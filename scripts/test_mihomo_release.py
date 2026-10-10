#!/usr/bin/env python3
import importlib.util
import gzip
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("mihomo-release.py")
spec = importlib.util.spec_from_file_location("mihomo_release", SCRIPT)
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def test_latest_release_rejects_unexpected_redirect(self):
        with patch.object(release, "curl", return_value="https://example.org/v1.19.32"):
            with self.assertRaises(ValueError):
                release.latest_release()

    def test_latest_release_accepts_stable_tag(self):
        url = "https://github.com/MetaCubeX/mihomo/releases/tag/v1.19.32"
        with patch.object(release, "curl", return_value=url):
            self.assertEqual(release.latest_release(), "v1.19.32")

    def test_asset_digest_requires_exact_asset(self):
        html = ('<clipboard-copy aria-label="Copy to clipboard digest for other.gz" '
                'value="sha256:' + '0' * 64 + '"></clipboard-copy>'
                '<clipboard-copy aria-label="Copy to clipboard digest for mihomo.gz" '
                'value="sha256:' + 'a' * 64 + '"></clipboard-copy>')
        with patch.object(release, "curl", return_value=html):
            self.assertEqual(release.upstream_digest("v1.19.32", "mihomo.gz"), 'a' * 64)
            with self.assertRaises(ValueError):
                release.upstream_digest("v1.19.32", "missing.gz")

    def test_numeric_version_comparison(self):
        self.assertGreater(release.version_parts("v1.19.32"), release.version_parts("v1.19.9"))
        with self.assertRaises(ValueError):
            release.version_parts("v1.19.32-alpha")

    def test_failed_digest_does_not_change_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / "manifest.json"
            manifest.write_text('{"version": "1.19.26"}\n')
            with patch.object(release, "upstream_digest", side_effect=ValueError("missing asset")):
                with self.assertRaises(ValueError):
                    release.update_manifest(manifest, {"version": "1.19.26"}, "v1.19.32")
            self.assertEqual(manifest.read_text(), '{"version": "1.19.26"}\n')

    def test_verified_archive_updates_pin(self):
        binary = b"test executable"
        archive = gzip.compress(binary)
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / "manifest.json"
            manifest.write_text('{"schema_version": 1, "version": "1.19.26", "targets": {}}\n')

            def fake_run(command, **kwargs):
                Path(command[command.index("--output") + 1]).write_bytes(archive)

            with patch.object(release, "upstream_digest", return_value=hashlib.sha256(archive).hexdigest()), \
                    patch.object(release.subprocess, "run", side_effect=fake_run):
                release.update_manifest(manifest, json.loads(manifest.read_text()), "v1.19.32")
            updated = json.loads(manifest.read_text())
            self.assertEqual(updated["version"], "1.19.32")
            self.assertEqual(updated["targets"][release.TARGET]["executable_sha256"],
                             hashlib.sha256(binary).hexdigest())


if __name__ == "__main__":
    unittest.main()
