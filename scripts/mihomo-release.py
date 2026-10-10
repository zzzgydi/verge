#!/usr/bin/env python3
"""Check the upstream stable release or explicitly advance the pinned sidecar."""

import argparse
import gzip
from html.parser import HTMLParser
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile


REPOSITORY = "https://github.com/MetaCubeX/mihomo"
TARGET = "aarch64-apple-darwin"
MAX_EXECUTABLE_SIZE = 128 * 1024 * 1024
VERSION_RE = re.compile(r"v(\d+)\.(\d+)\.(\d+)")
SHA256_RE = re.compile(r"sha256:([0-9a-f]{64})\Z")


def curl(*args):
    return subprocess.check_output(
        ["curl", "--fail", "--silent", "--show-error", "--location",
         "--connect-timeout", "10", "--max-time", "30", *args],
        stderr=subprocess.PIPE,
    ).decode("utf-8").strip()


def latest_release():
    url = curl("--output", "/dev/null", "--write-out", "%{url_effective}",
               f"{REPOSITORY}/releases/latest")
    match = re.fullmatch(rf"{re.escape(REPOSITORY)}/releases/tag/(v\d+\.\d+\.\d+)", url)
    if not match:
        raise ValueError(f"unexpected latest release URL: {url}")
    return match.group(1)


def version_parts(version):
    match = VERSION_RE.fullmatch(version)
    if not match:
        raise ValueError(f"unsupported Mihomo version: {version}")
    return tuple(map(int, match.groups()))


class AssetDigestParser(HTMLParser):
    def __init__(self, asset):
        super().__init__()
        self.asset = asset
        self.digests = []

    def handle_starttag(self, tag, attrs):
        if tag != "clipboard-copy":
            return
        attrs = dict(attrs)
        if attrs.get("aria-label") != f"Copy to clipboard digest for {self.asset}":
            return
        digest = SHA256_RE.fullmatch(attrs.get("value", ""))
        if digest:
            self.digests.append(digest.group(1))


def upstream_digest(tag, asset):
    parser = AssetDigestParser(asset)
    parser.feed(curl(f"{REPOSITORY}/releases/expanded_assets/{tag}"))
    if len(parser.digests) != 1:
        raise ValueError(f"expected exactly one upstream SHA-256 for {asset}")
    return parser.digests[0]


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def update_manifest(manifest_path, manifest, tag):
    asset = f"mihomo-darwin-arm64-{tag}.gz"
    archive_digest = upstream_digest(tag, asset)
    download_url = f"{REPOSITORY}/releases/download/{tag}/{asset}"
    with tempfile.TemporaryDirectory(prefix="verge-mihomo-") as directory:
        archive = Path(directory) / asset
        executable = Path(directory) / "mihomo"
        subprocess.run(
            ["curl", "--fail", "--show-error", "--location", "--retry", "3",
             "--connect-timeout", "20", "--output", str(archive), download_url],
            check=True, stdout=subprocess.DEVNULL,
        )
        if sha256(archive) != archive_digest:
            raise ValueError("downloaded archive does not match upstream SHA-256")
        with gzip.open(archive, "rb") as source, executable.open("wb") as destination:
            remaining = MAX_EXECUTABLE_SIZE
            while chunk := source.read(min(1024 * 1024, remaining + 1)):
                remaining -= len(chunk)
                if remaining < 0:
                    raise ValueError("extracted Mihomo exceeds 128 MiB")
                destination.write(chunk)
        updated = dict(manifest)
        updated["version"] = tag[1:]
        updated["release_url"] = f"{REPOSITORY}/releases/tag/{tag}"
        updated["targets"] = dict(manifest["targets"])
        updated["targets"][TARGET] = {
            "asset": asset,
            "download_url": download_url,
            "archive_sha256": archive_digest,
            "executable_sha256": sha256(executable),
        }
        # Keep the old pin intact until both upstream digest and binary are verified.
        with tempfile.NamedTemporaryFile("w", dir=manifest_path.parent, delete=False) as output:
            staged = Path(output.name)
            try:
                json.dump(updated, output, indent=2)
                output.write("\n")
                output.flush()
                os.fchmod(output.fileno(), manifest_path.stat().st_mode & 0o777)
            except BaseException:
                staged.unlink(missing_ok=True)
                raise
        os.replace(staged, manifest_path)
    print(f"Pinned Mihomo {tag} in {manifest_path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("check", "update"))
    parser.add_argument("--manifest", type=Path,
                        default=Path(os.environ.get("VERGE_MIHOMO_MANIFEST") or
                                     Path(__file__).resolve().parent.parent /
                                     "assets/mihomo/manifest.json"))
    args = parser.parse_args()
    try:
        manifest = json.loads(args.manifest.read_text())
        pinned = version_parts("v" + manifest["version"])
        tag = latest_release()
        latest = version_parts(tag)
        if latest <= pinned:
            print(f"Mihomo pin v{manifest['version']} is up to date (upstream: {tag}).", file=sys.stderr)
            return 0
        if args.action == "check":
            print(f"Mihomo {tag} is available; build still uses pinned v{manifest['version']}. "
                  "Run `make mihomo-upgrade` to verify and pin it.", file=sys.stderr)
        else:
            update_manifest(args.manifest, manifest, tag)
        return 0
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        if args.action == "check":
            print(f"warning: unable to check upstream Mihomo release: {error}", file=sys.stderr)
            return 0
        print(f"error: Mihomo upgrade failed; pin unchanged: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
