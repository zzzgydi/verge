#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
app_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
repo_dir=$(CDPATH= cd -- "$app_dir/../.." && pwd)
target_arch=$(uname -m)

if [ "$target_arch" != "arm64" ]; then
    echo "Phase 1 packaging supports Apple Silicon only; found $target_arch" >&2
    exit 1
fi

if [ -z "${VERGE_MIHOMO_BIN:-}" ] || [ ! -f "$VERGE_MIHOMO_BIN" ]; then
    echo "Set VERGE_MIHOMO_BIN to the pinned Mihomo v1.19.26 executable" >&2
    exit 1
fi

manifest="$repo_dir/assets/mihomo/manifest.json"
expected=$(/usr/bin/plutil -extract targets.aarch64-apple-darwin.executable_sha256 raw "$manifest")
actual=$(/usr/bin/shasum -a 256 "$VERGE_MIHOMO_BIN" | /usr/bin/awk '{print $1}')
if [ "$actual" != "$expected" ]; then
    echo "Mihomo executable SHA-256 does not match the pinned manifest" >&2
    exit 1
fi

cargo build --manifest-path "$repo_dir/Cargo.toml" -p verge --bin verge-gpui --release --locked
cargo build --manifest-path "$repo_dir/Cargo.toml" -p verge-helper --release --locked

bundle="$repo_dir/dist/Verge.app"
contents="$bundle/Contents"
/bin/rm -rf "$bundle"
/bin/mkdir -p "$contents/MacOS" "$contents/Resources/bin" "$contents/Resources/helper"
/bin/cp "$repo_dir/target/release/verge-gpui" "$contents/MacOS/verge-gpui"
/bin/cp "$VERGE_MIHOMO_BIN" "$contents/Resources/bin/mihomo"
/bin/cp "$repo_dir/target/release/verge-helper" "$contents/Resources/helper/verge-helper"
/bin/cp "$repo_dir/assets/icons/icon.icns" "$contents/Resources/AppIcon.icns"
/bin/chmod 755 "$contents/MacOS/verge-gpui" "$contents/Resources/bin/mihomo"
/bin/chmod 755 "$contents/Resources/helper/verge-helper"
# 构建时记录 helper 预期 SHA-256，安装时按此校验来源二进制。
/usr/bin/shasum -a 256 "$contents/Resources/helper/verge-helper" | /usr/bin/awk '{print $1}' > "$contents/Resources/helper/verge-helper.sha256"
/bin/cp "$manifest" "$contents/Resources/mihomo-manifest.json"
/bin/cp "$app_dir/macos/Info.plist" "$contents/Info.plist"
# 版本号以 Cargo.toml 为准，打包时写入 plist；应用内更新据此报告/比较当前版本。
version=$(/usr/bin/sed -n 's/^version = "\(.*\)"/\1/p' "$app_dir/Cargo.toml" | /usr/bin/head -1)
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $version" "$contents/Info.plist"

signing_identity=${VERGE_CODESIGN_IDENTITY:--}
/usr/bin/codesign --force --deep --options runtime --sign "$signing_identity" "$bundle"
/usr/bin/codesign --verify --deep --strict --verbose=2 "$bundle"

echo "$bundle"
