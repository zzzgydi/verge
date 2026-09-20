#!/bin/sh
set -eu
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
# create-dmg v1.3.0. Verify the archive before extracting any executable code.
revision=a2b71d0fda6d0df2a86dc7f67082d4d73e84c59f
digest=36577b966f16c12dd78d5bb5107c2ae3d069b044226b6ebbffa6a434ce142d0a
cache="$repo_dir/.cache/create-dmg"
mkdir -p "$cache"
archive="$cache/$revision.tar.gz"
verify() {
    [ -f "$archive" ] && [ "$(shasum -a 256 "$archive" | awk '{print $1}')" = "$digest" ]
}
if ! verify; then
    download=$(mktemp "$cache/.download.XXXXXX")
    trap 'rm -f "$download"' EXIT
    trap 'exit 1' HUP INT TERM
    curl --fail --location --retry 3 --connect-timeout 20 \
        --output "$download" "https://codeload.github.com/create-dmg/create-dmg/tar.gz/$revision"
    [ "$(shasum -a 256 "$download" | awk '{print $1}')" = "$digest" ] || {
        echo 'create-dmg archive SHA-256 mismatch' >&2; exit 1;
    }
    mv "$download" "$archive"
    trap - EXIT HUP INT TERM
fi
# Re-extract the verified archive rather than trust modified cached scripts.
tar -xzf "$archive" -C "$cache"
printf '%s\n' "$cache/create-dmg-$revision/create-dmg"
