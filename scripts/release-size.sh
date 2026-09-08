#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
bundle="$repo_dir/dist/Verge.app"
archive="$repo_dir/dist/Verge-macos-arm64.zip"

if [ ! -d "$bundle" ] || [ ! -f "$archive" ]; then
    echo "No packaged release found. Run make release first." >&2
    exit 1
fi

target_dir=$(cargo +1.97.1 metadata --manifest-path "$repo_dir/Cargo.toml" --no-deps --format-version 1 | jq -r '.target_directory')
file_size() {
    if [ -f "$2" ]; then
        bytes=$(/usr/bin/stat -f '%z' "$2")
        /usr/bin/awk -v label="$1" -v bytes="$bytes" 'BEGIN { printf "%-28s %8.2f MiB\n", label, bytes / 1048576 }'
    fi
}

echo "File sizes (not runtime memory):"
file_size "Debug executable (if built)" "$target_dir/debug/verge-gpui"
file_size "Release executable (Cargo)" "$target_dir/release/verge-gpui"
file_size "App executable (stripped)" "$bundle/Contents/MacOS/verge-gpui"
file_size "Bundled Mihomo" "$bundle/Contents/Resources/bin/mihomo"
file_size "Bundled helper" "$bundle/Contents/Resources/helper/verge-helper"
file_size "ZIP archive" "$archive"
size_kib=$(/usr/bin/du -sk "$bundle" | /usr/bin/awk '{print $1}')
/usr/bin/awk -v kib="$size_kib" 'BEGIN { printf "%-28s %8.2f MiB\n", "Verge.app (disk usage)", kib / 1024 }'
