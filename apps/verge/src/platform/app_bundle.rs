//! 当前运行实例的 `.app` bundle 定位与版本读取（应用自身更新用）。
//!
//! 版本来源：`apps/verge/scripts/build-macos-app.sh` 在打包时把
//! `apps/verge/Cargo.toml` 的 version 写入 `Contents/Info.plist` 的
//! `CFBundleShortVersionString`；运行时读本进程 bundle 的 plist，
//! 非 `.app` 运行（开发模式、测试）返回 None/可读错误，由上层提示。

use std::{
    ffi::CString,
    fs,
    path::{Path, PathBuf},
};

use crate::domain::{AppError, ErrorCode};

/// 当前进程若位于 `.app` 内（`*.app/Contents/MacOS/<exe>`），返回 bundle 根路径。
pub fn current_app_bundle() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then(|| bundle.to_owned())
}

/// 读取 `.app` 的 `CFBundleShortVersionString`。
///
/// 构建脚本产出的 Info.plist 是固定模板的 XML plist，这里做最小的
/// `<key>…</key>` → 紧随的 `<string>…</string>` 提取，不引入 plist 解析依赖。
pub fn bundle_short_version(bundle: &Path) -> Result<String, AppError> {
    let plist = bundle.join("Contents/Info.plist");
    let text = fs::read_to_string(&plist).map_err(|error| {
        AppError::new(
            ErrorCode::PlatformFailed,
            format!("cannot read {}: {error}", plist.display()),
        )
    })?;
    plist_string_value(&text, "CFBundleShortVersionString").ok_or_else(|| {
        AppError::new(
            ErrorCode::PlatformFailed,
            format!("{} has no CFBundleShortVersionString", plist.display()),
        )
    })
}

/// 从 XML plist 文本中提取指定 key 之后第一个 `<string>` 值。
fn plist_string_value(text: &str, key: &str) -> Option<String> {
    let key_marker = format!("<key>{key}</key>");
    let start = text.find(&key_marker)? + key_marker.len();
    let rest = &text[start..];
    let value_start = rest.find("<string>")? + "<string>".len();
    let value_end = rest[value_start..].find("</string>")? + value_start;
    let value = rest[value_start..value_end].trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// 目录对当前真实用户是否可写（决定 .app 替换走直接 rename 还是提权路径）。
/// 用 `access(2)` 的真实 UID 语义，而不是只看权限位。
pub fn directory_writable(directory: &Path) -> bool {
    let Ok(path) = CString::new(directory.as_os_str().as_encoded_bytes()) else {
        return false;
    };
    // SAFETY: access() with a valid C string has no preconditions.
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "verge-platform-app-bundle-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn reads_short_version_from_bundled_plist() {
        let dir = test_dir("version");
        let bundle = dir.join("Verge.app");
        fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        fs::write(
            bundle.join("Contents/Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Verge</string>
    <key>CFBundleShortVersionString</key>
    <string>0.2.1</string>
</dict>
</plist>
"#,
        )
        .unwrap();

        assert_eq!(bundle_short_version(&bundle).unwrap(), "0.2.1");

        fs::write(
            bundle.join("Contents/Info.plist"),
            "<plist><dict></dict></plist>",
        )
        .unwrap();
        assert_eq!(
            bundle_short_version(&bundle).unwrap_err().code,
            ErrorCode::PlatformFailed
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn temp_directory_is_writable_and_missing_path_is_not() {
        let dir = test_dir("writable");
        assert!(directory_writable(&dir));
        assert!(!directory_writable(&dir.join("does-not-exist")));
        let _ = fs::remove_dir_all(dir);
    }
}
