#!/bin/sh
set -eu
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
bundle="$repo_dir/dist/Verge.app"
[ -d "$bundle" ] || { echo 'Build Verge.app first.' >&2; exit 1; }
create_dmg=$("$script_dir/ensure-create-dmg.sh")
staging=$(mktemp -d "$repo_dir/dist/.verge-dmg.XXXXXX")
cleanup() {
    # Detach only disk images created in this invocation's private directory.
    hdiutil info -plist | plutil -convert json -o - - | \
        jq -r --arg prefix "$staging/" '.images[]? | select(."image-path" | startswith($prefix)) | ."system-entities"[]? | select(."mount-point") | ."dev-entry"' | \
        while IFS= read -r device; do hdiutil detach "$device" >/dev/null 2>&1 || true; done
    rm -rf "$staging"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
mkdir "$staging/source"
ditto "$bundle" "$staging/source/Verge.app"
"$create_dmg" --volname Verge --volicon "$repo_dir/assets/icons/icon.icns" \
    --background "$repo_dir/assets/dmg/background.png" \
    --window-pos 200 160 --window-size 640 420 --icon-size 112 \
    --text-size 14 --icon Verge.app 170 185 --hide-extension Verge.app \
    --app-drop-link 470 185 --no-internet-enable \
    "$staging/Verge-macos-arm64.dmg" "$staging/source"
hdiutil verify "$staging/Verge-macos-arm64.dmg"
mv "$staging/Verge-macos-arm64.dmg" "$repo_dir/dist/Verge-macos-arm64.dmg"
(cd "$repo_dir/dist" && shasum -a 256 Verge-macos-arm64.dmg > Verge-macos-arm64.dmg.sha256)
echo "$repo_dir/dist/Verge-macos-arm64.dmg"
