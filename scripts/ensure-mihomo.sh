#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
manifest=${VERGE_MIHOMO_MANIFEST:-"$repo_dir/assets/mihomo/manifest.json"}

fail() {
    echo "error: $*" >&2
    exit 1
}

command -v jq >/dev/null 2>&1 || fail "jq is required"
command -v curl >/dev/null 2>&1 || fail "curl is required"
command -v shasum >/dev/null 2>&1 || fail "shasum is required"
[ -f "$manifest" ] || fail "Mihomo manifest not found: $manifest"

case "$(uname -s):$(uname -m)" in
    Darwin:arm64) target="aarch64-apple-darwin" ;;
    *) fail "unsupported development platform: $(uname -s) $(uname -m)" ;;
esac

read_manifest() {
    jq -er --arg target "$target" ".targets[\$target].$1" "$manifest"
}

version=$(jq -er '.version' "$manifest")
asset=$(read_manifest asset)
download_url=$(read_manifest download_url)
archive_sha256=$(read_manifest archive_sha256)
executable_sha256=$(read_manifest executable_sha256)

if [ -n "${VERGE_MIHOMO_BIN:-}" ]; then
    mihomo_bin=$VERGE_MIHOMO_BIN
    managed=0
else
    cache_dir=${VERGE_MIHOMO_CACHE_DIR:-"$repo_dir/.cache/mihomo"}
    mihomo_bin="$cache_dir/$target/mihomo"
    managed=1
fi

verify_file() {
    file=$1
    expected=$2
    [ -f "$file" ] || return 1
    actual=$(shasum -a 256 "$file" | awk '{print $1}')
    [ "$actual" = "$expected" ]
}

if verify_file "$mihomo_bin" "$executable_sha256"; then
    echo "Mihomo v$version is ready: $mihomo_bin" >&2
    printf '%s\n' "$mihomo_bin"
    exit 0
fi

if [ "$managed" -eq 0 ]; then
    fail "VERGE_MIHOMO_BIN is missing or does not match the manifest: $mihomo_bin"
fi

if [ -e "$mihomo_bin" ]; then
    echo "Replacing Mihomo because its SHA-256 does not match the manifest: $mihomo_bin" >&2
fi

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/verge-mihomo.XXXXXX")
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM
archive="$tmp_dir/$asset"
candidate="$tmp_dir/mihomo"

echo "Downloading Mihomo v$version for $target..." >&2
curl --fail --location --retry 3 --retry-delay 1 --connect-timeout 20 \
    --output "$archive" "$download_url"
verify_file "$archive" "$archive_sha256" || fail "downloaded Mihomo archive SHA-256 mismatch"

case "$asset" in
    *.gz) gzip -dc "$archive" > "$candidate" ;;
    *) fail "unsupported Mihomo archive format: $asset" ;;
esac
verify_file "$candidate" "$executable_sha256" || fail "extracted Mihomo SHA-256 mismatch"

mkdir -p "$(dirname -- "$mihomo_bin")"
chmod 755 "$candidate"
mv "$candidate" "$mihomo_bin"
echo "Installed Mihomo v$version: $mihomo_bin" >&2
printf '%s\n' "$mihomo_bin"
