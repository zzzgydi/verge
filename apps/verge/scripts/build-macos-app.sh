#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
app_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
repo_dir=$(CDPATH= cd -- "$app_dir/../.." && pwd)
dist_dir=${VERGE_DIST_DIR:-"$repo_dir/dist"}
target_arch=$(uname -m)
export MACOSX_DEPLOYMENT_TARGET=15.0

case "$dist_dir" in
    /*) ;;
    *) echo "VERGE_DIST_DIR must be an absolute path" >&2; exit 1 ;;
esac

if [ "$target_arch" != "arm64" ]; then
    echo "Phase 1 packaging supports Apple Silicon only; found $target_arch" >&2
    exit 1
fi

mihomo_bin=$("$repo_dir/scripts/ensure-mihomo.sh")

manifest="$repo_dir/assets/mihomo/manifest.json"
expected=$(/usr/bin/plutil -extract targets.aarch64-apple-darwin.executable_sha256 raw "$manifest")
actual=$(/usr/bin/shasum -a 256 "$mihomo_bin" | /usr/bin/awk '{print $1}')
if [ "$actual" != "$expected" ]; then
    echo "Mihomo executable SHA-256 does not match the pinned manifest" >&2
    exit 1
fi

VERGE_BUILD_CHANNEL=stable cargo +1.97.1 build --manifest-path "$repo_dir/Cargo.toml" -p verge --bin verge-gpui --release --locked
cargo +1.97.1 build --manifest-path "$repo_dir/Cargo.toml" -p verge-helper --release --locked
# Resolve Cargo's actual output directory, including CARGO_TARGET_DIR overrides.
target_dir=$(cargo +1.97.1 metadata --manifest-path "$repo_dir/Cargo.toml" --no-deps --format-version 1 | jq -r '.target_directory')

bundle="$dist_dir/Verge.app"
contents="$bundle/Contents"
/bin/mkdir -p "$dist_dir"
/bin/rm -rf "$bundle"
/bin/mkdir -p "$contents/MacOS" "$contents/Resources/bin" "$contents/Resources/helper"
/bin/cp "$target_dir/release/verge-gpui" "$contents/MacOS/verge-gpui"
/bin/cp "$mihomo_bin" "$contents/Resources/bin/mihomo"
/bin/cp "$target_dir/release/verge-helper" "$contents/Resources/helper/verge-helper"
/bin/cp "$repo_dir/assets/icons/icon.icns" "$contents/Resources/AppIcon.icns"
/bin/chmod 755 "$contents/MacOS/verge-gpui" "$contents/Resources/bin/mihomo"
/bin/chmod 755 "$contents/Resources/helper/verge-helper"
/bin/cp "$manifest" "$contents/Resources/mihomo-manifest.json"
/bin/cp "$app_dir/macos/Info.plist" "$contents/Info.plist"
# 版本号以 Cargo.toml 为准，打包时写入 plist；应用内更新据此报告/比较当前版本。
version=$(/usr/bin/sed -n 's/^version = "\(.*\)"/\1/p' "$app_dir/Cargo.toml" | /usr/bin/head -1)
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $version" "$contents/Info.plist"

signing_identity=${VERGE_CODESIGN_IDENTITY:--}
# Strip only the bundled Rust binaries; preserve Cargo outputs for profiling.
/usr/bin/strip -x "$contents/MacOS/verge-gpui" "$contents/Resources/helper/verge-helper"
# Sign inside out. Keep Mihomo's upstream signature so its pinned SHA-256 stays valid.
/usr/bin/codesign --force --options runtime --sign "$signing_identity" "$contents/Resources/helper/verge-helper"
# Signing changes the binary; record the digest of the final helper that will be installed.
/usr/bin/shasum -a 256 "$contents/Resources/helper/verge-helper" | /usr/bin/awk '{print $1}' > "$contents/Resources/helper/verge-helper.sha256"
/usr/bin/codesign --force --options runtime --sign "$signing_identity" "$bundle"
/usr/bin/codesign --verify --deep --strict --verbose=2 "$bundle"

bundle_min=$(/usr/bin/plutil -extract LSMinimumSystemVersion raw "$contents/Info.plist")
if [ "$bundle_min" != "15.0" ]; then
    echo "Unexpected application minimum macOS version: $bundle_min" >&2
    exit 1
fi
for binary in "$contents/MacOS/verge-gpui" "$contents/Resources/helper/verge-helper"; do
    minimum=$(xcrun vtool -show-build "$binary" | /usr/bin/awk '/ minos / { print $2; exit }')
    if [ "$minimum" != "15.0" ]; then
        echo "Unsupported deployment target for $binary: ${minimum:-unknown}" >&2
        exit 1
    fi
done
minimum=$(xcrun vtool -show-build "$contents/Resources/bin/mihomo" | /usr/bin/awk '/ minos / { print $2; exit }')
if [ -z "$minimum" ] || ! /usr/bin/awk -v min="$minimum" 'BEGIN { exit !(min + 0 <= 15.0) }'; then
    echo "Unsupported deployment target for Mihomo: ${minimum:-unknown}" >&2
    exit 1
fi

actual=$(/usr/bin/shasum -a 256 "$contents/Resources/bin/mihomo" | /usr/bin/awk '{print $1}')
if [ "$actual" != "$expected" ]; then
    echo "Bundled Mihomo no longer matches the pinned manifest" >&2
    exit 1
fi

archive="$dist_dir/Verge-macos-arm64.zip"
/bin/rm -f "$archive"
/usr/bin/ditto -c -k --sequesterRsrc --keepParent "$bundle" "$archive"
(cd "$dist_dir" && /usr/bin/shasum -a 256 Verge-macos-arm64.zip > Verge-macos-arm64.zip.sha256)

echo "$bundle"
echo "$archive"
VERGE_DIST_DIR="$dist_dir" "$repo_dir/scripts/build-dmg.sh"
