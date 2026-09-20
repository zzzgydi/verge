#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
bundle="$repo_dir/dist/Verge Dev.app"

if [ "${1:-}" = "--stop" ]; then
    if [ -x "$bundle/Contents/MacOS/verge-gpui" ]; then
        exec "$bundle/Contents/MacOS/verge-gpui" --dev-stop
    fi
    echo "No Verge Dev bundle found; run make dev first." >&2
    exit 0
fi

mode=run
if [ "${1:-}" = "--build-only" ]; then mode=build; shift; fi
profile=${VERGE_DEV_PROFILE:-dev}
case "$profile" in
    dev) output=debug ;;
    release) output=release ;;
    *) echo "VERGE_DEV_PROFILE must be dev or release" >&2; exit 1 ;;
esac
export VERGE_BUILD_CHANNEL=dev
mihomo_bin=$("$script_dir/ensure-mihomo.sh")
cd "$repo_dir"
cargo +1.97.1 build --manifest-path "$repo_dir/Cargo.toml" -p verge --bin verge-gpui --profile "$profile" --locked
target_dir=$(cargo +1.97.1 metadata --manifest-path "$repo_dir/Cargo.toml" --no-deps --format-version 1 | jq -r '.target_directory')

# Assemble and verify the replacement before asking the old Dev daemon to quit.
mkdir -p "$repo_dir/dist"
staging=$(mktemp -d "$repo_dir/dist/.verge-dev.XXXXXX")
trap 'rm -rf "$staging"' EXIT
trap 'exit 1' HUP INT TERM
candidate="$staging/Verge Dev.app"
contents="$candidate/Contents"
mkdir -p "$contents/MacOS" "$contents/Resources/bin"
cp "$target_dir/$output/verge-gpui" "$contents/MacOS/verge-gpui"
cp "$mihomo_bin" "$contents/Resources/bin/mihomo"
cp "$repo_dir/assets/mihomo/manifest.json" "$contents/Resources/mihomo-manifest.json"
cp "$repo_dir/assets/icons/dev.icns" "$contents/Resources/AppIcon.icns"
cp "$repo_dir/apps/verge/macos/Info.plist" "$contents/Info.plist"
/usr/libexec/PlistBuddy -c 'Set :CFBundleIdentifier com.zzzgydi.verge.dev' "$contents/Info.plist"
/usr/libexec/PlistBuddy -c 'Set :CFBundleName Verge Dev' "$contents/Info.plist"
/usr/libexec/PlistBuddy -c 'Set :CFBundleDisplayName Verge Dev' "$contents/Info.plist"
version=$(/usr/bin/sed -n 's/^version = "\(.*\)"/\1/p' "$repo_dir/apps/verge/Cargo.toml" | /usr/bin/head -1)
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $version" "$contents/Info.plist"
/usr/bin/codesign --force --sign - "$candidate"
/usr/bin/codesign --verify --deep --strict "$candidate"

# The Dev-only maintenance handshake also works while its GUI is connected.
# A failed build or stop never replaces the existing bundle.
"$contents/MacOS/verge-gpui" --dev-stop
rm -rf "$bundle"
mv "$candidate" "$bundle"
rm -rf "$staging"
trap - EXIT HUP INT TERM
echo "Built $bundle"
if [ "$mode" = build ]; then exit 0; fi
exec "$bundle/Contents/MacOS/verge-gpui" "$@"
