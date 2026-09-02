//! 应用自身更新：从 GitHub Releases 检查、下载、校验并替换当前 `Verge.app`。
//!
//! ## 发布资产约定（发布仓库 `zzzgydi/verge` 的 GitHub Release）
//!
//! - tag 为语义化版本，允许 `v` 前缀（如 `v0.2.0`）；草稿与预发布不参与更新；
//! - macOS arm64 资产固定命名 `Verge-macos-arm64.zip`，zip 解压后顶层有且
//!   仅有目标 bundle `Verge.app`；
//! - 同名旁车摘要 `Verge-macos-arm64.zip.sha256`，内容为 `shasum -a 256`
//!   输出格式：`<64 位 hex>` 后可跟空白与文件名。
//!
//! ## 事务顺序
//!
//! 受限下载器下载 release JSON / 摘要 / zip → SHA-256 校验 zip → 解压到数据
//! 目录 staged → 校验 bundle 结构（`Contents/MacOS/verge-gpui` 存在、
//! Info.plist 版本与 release 一致）→ `codesign --verify --deep --strict`
//! 深度校验（覆盖主可执行与内置 helper）→ 签名身份与当前运行实例一致
//! （同源策略，见下）→ 备份当前 .app 到数据目录 `previous/` → 原子替换
//! （目标父目录可写时直接同卷 rename；不可写时经 osascript 管理员授权脚本，
//! 脚本内置失败回滚）→ 返回“重启生效”。
//!
//! 替换路径上的任何失败都不破坏现有安装：直接路径 rename 失败即改名还原，
//! 提权脚本在目标缺失且旧版本尚存时自动 `mv` 回滚。
//!
//! ## 签名校验策略
//!
//! 深度校验保证 bundle 内所有可执行签名有效；身份比较要求候选与当前实例的
//! 主签名身份完全一致（`codesign -dv` 输出的首个 `Authority=`；ad-hoc 签名
//! 记为 `adhoc`）。当前实例签名校验失败或身份不一致都拒绝更新——开发期的
//! ad-hoc 构建只能被 ad-hoc 构建替换，正式签名构建同理，杜绝跨来源覆盖。
//!
//! ## 重启编排
//!
//! `UpdateApplication` 只完成替换并返回 `restart_required: true`；由用户确认
//! 后发送 `RestartApplication`，守护进程先拉起新 bundle 的可执行、再退出
//! 自身（GUI 随 IPC 断开退出，新进程按既有 connect-or-spawn 逻辑接管）。
//! 选这个“下载完成，重启生效”的两步流程而不是原地热替换：正在运行的进程
//! 无法安全替换自身二进制映像，且双进程（守护 + GUI）的退出顺序交给用户
//! 显式触发最可控。新版启动失败的在线检测/自动回滚超出本期范围；数据目录
//! 保留的 `previous/Verge.app` 是人工恢复兜底。

use std::{fmt, fs, path::Path};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use crate::domain::{AppError, AppUpdateStatus, ErrorCode};
use crate::platform::{
    CommandRunner, bundle_short_version, directory_writable, run_privileged_script, shell_quote,
};

use super::ArtifactFetcher;

/// GitHub Releases 最新 release 的 API 地址（发布仓库 zzzgydi/verge）。
pub const APP_RELEASE_API_URL: &str = "https://api.github.com/repos/zzzgydi/verge/releases/latest";
/// macOS arm64 更新资产名（zip 内含顶层 `Verge.app`）。
pub const APP_ASSET_NAME: &str = "Verge-macos-arm64.zip";
/// zip 内目标 bundle 名。
pub const APP_BUNDLE_NAME: &str = "Verge.app";
/// bundle 主可执行名（与 `CFBundleExecutable` 一致）。
pub const APP_EXECUTABLE_NAME: &str = "verge-gpui";
/// 资产下载 URL 必须位于发布仓库的 release 下载路径下。
const RELEASE_DOWNLOAD_PREFIX: &str = "https://github.com/zzzgydi/verge/releases/download/";

/// 最小语义化版本：允许 `v` 前缀，2–3 个点分数字段，不接受预发布/构建后缀
/// （预发布应通过 GitHub 的 prerelease 标记排除，不参与比较）。
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SemanticVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl SemanticVersion {
    pub fn parse(value: &str) -> Result<Self, AppError> {
        let invalid = || {
            AppError::new(
                ErrorCode::ValidationFailed,
                format!(
                    "version '{value}' is not a semantic version (expected [v]major.minor[.patch])"
                ),
            )
        };
        let value = value.trim();
        let value = value.strip_prefix(['v', 'V']).unwrap_or(value);
        let parts = value.split('.').collect::<Vec<_>>();
        if !(2..=3).contains(&parts.len()) {
            return Err(invalid());
        }
        let mut numbers = [0_u64; 3];
        for (slot, part) in numbers.iter_mut().zip(parts.iter()) {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(invalid());
            }
            *slot = part.parse::<u64>().map_err(|_| invalid())?;
        }
        Ok(Self {
            major: numbers[0],
            minor: numbers[1],
            patch: numbers[2],
        })
    }
}

impl fmt::Display for SemanticVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// 从最新 release 中选出的更新目标。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppUpdateRelease {
    pub version: SemanticVersion,
    pub tag: String,
    pub asset_url: String,
    pub sha256_url: String,
}

#[derive(Deserialize)]
struct ReleaseJson {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<ReleaseAssetJson>,
}

#[derive(Deserialize)]
struct ReleaseAssetJson {
    name: String,
    browser_download_url: String,
}

/// 解析 GitHub `releases/latest` 响应并选出 macOS arm64 资产与其摘要。
pub fn parse_latest_release(json: &[u8]) -> Result<AppUpdateRelease, AppError> {
    let release: ReleaseJson = serde_json::from_slice(json).map_err(|error| {
        AppError::new(
            ErrorCode::CoreUnavailable,
            format!("cannot parse the GitHub release response: {error}"),
        )
    })?;
    if release.draft || release.prerelease {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "the latest GitHub release is a draft or pre-release; skipping update check",
        ));
    }
    let version = SemanticVersion::parse(&release.tag_name)?;
    let tag = release.tag_name.clone();
    let asset = find_asset(&release, APP_ASSET_NAME)?;
    let sha256 = find_asset(&release, &format!("{APP_ASSET_NAME}.sha256"))?;
    for url in [&asset.browser_download_url, &sha256.browser_download_url] {
        if !url.starts_with(RELEASE_DOWNLOAD_PREFIX) {
            return Err(AppError::new(
                ErrorCode::ValidationFailed,
                format!("release asset URL is not on the official release host: {url}"),
            ));
        }
    }
    Ok(AppUpdateRelease {
        version,
        tag,
        asset_url: asset.browser_download_url.clone(),
        sha256_url: sha256.browser_download_url.clone(),
    })
}

fn find_asset<'a>(release: &'a ReleaseJson, name: &str) -> Result<&'a ReleaseAssetJson, AppError> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .ok_or_else(|| {
            AppError::new(
                ErrorCode::NotFound,
                format!("release {} has no asset named {name}", release.tag_name),
            )
        })
}

/// 解析 `.sha256` 旁车文件：首个空白分隔字段必须是 64 位 hex 摘要。
pub fn parse_sha256_sidecar(text: &str) -> Result<String, AppError> {
    let invalid = || {
        AppError::new(
            ErrorCode::ValidationFailed,
            "the release .sha256 asset does not start with a 64-hex digest",
        )
    };
    let digest = text.split_whitespace().next().ok_or_else(invalid)?;
    let digest = digest.to_ascii_lowercase();
    if digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(digest)
    } else {
        Err(invalid())
    }
}

/// 检查应用更新：拉取最新 release 并与当前版本（None = 非 .app 运行）比较。
pub fn check_app_update<F: ArtifactFetcher>(
    fetcher: &mut F,
    current_version: Option<&str>,
) -> Result<AppUpdateStatus, AppError> {
    let release = parse_latest_release(&fetcher.fetch(APP_RELEASE_API_URL)?)?;
    let current = current_version.map(SemanticVersion::parse).transpose()?;
    Ok(AppUpdateStatus {
        current_version: current.map(|version| version.to_string()),
        update_available: current.is_some_and(|current| release.version > current),
        latest_version: release.version.to_string(),
        asset_url: release.asset_url,
    })
}

/// bundle 文件操作抽象：解压 zip、整树拷贝、整树删除。生产实现走
/// `/usr/bin/ditto` / `/bin/rm`（参数化、无 shell），测试注入 fake。
pub trait BundleArchiver {
    fn extract_zip(&mut self, archive: &Path, destination: &Path) -> Result<(), AppError>;
    fn copy_tree(&mut self, source: &Path, destination: &Path) -> Result<(), AppError>;
    /// 删除整树；目标不存在视为成功（幂等清理）。
    fn remove_tree(&mut self, path: &Path) -> Result<(), AppError>;
}

/// 用系统 ditto/rm 的 `BundleArchiver` 实现，命令经 `CommandRunner` 参数化执行。
pub struct DittoArchiver<R> {
    runner: R,
}

impl<R: CommandRunner> DittoArchiver<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }
}

impl<R: CommandRunner> BundleArchiver for DittoArchiver<R> {
    fn extract_zip(&mut self, archive: &Path, destination: &Path) -> Result<(), AppError> {
        self.runner
            .run(
                "/usr/bin/ditto",
                &[
                    "-x".into(),
                    "-k".into(),
                    path_arg(archive)?,
                    path_arg(destination)?,
                ],
            )
            .map(|_| ())
    }

    fn copy_tree(&mut self, source: &Path, destination: &Path) -> Result<(), AppError> {
        self.runner
            .run(
                "/usr/bin/ditto",
                &[path_arg(source)?, path_arg(destination)?],
            )
            .map(|_| ())
    }

    fn remove_tree(&mut self, path: &Path) -> Result<(), AppError> {
        self.runner
            .run("/bin/rm", &["-rf".into(), path_arg(path)?])
            .map(|_| ())
    }
}

/// 代码签名验证抽象：深度校验 + 签名身份提取。
pub trait CodeSignVerifier {
    /// `codesign --verify --deep --strict`；签名无效返回校验错误。
    fn verify_bundle(&mut self, app: &Path) -> Result<(), AppError>;
    /// 签名身份：首个 `Authority=` 值；ad-hoc 签名为 `"adhoc"`。
    fn signing_identity(&mut self, app: &Path) -> Result<String, AppError>;
}

/// `codesign --verify` 的参数组装（纯函数，供测试锁定命令形态）。
pub fn codesign_verify_args(app: &Path) -> Result<Vec<String>, AppError> {
    Ok(vec![
        "--verify".into(),
        "--deep".into(),
        "--strict".into(),
        path_arg(app)?,
    ])
}

/// `codesign -dv` 的参数组装（身份信息写在 stderr）。
pub fn codesign_identity_args(app: &Path) -> Result<Vec<String>, AppError> {
    Ok(vec!["-dv".into(), "--verbose=4".into(), path_arg(app)?])
}

/// 从 `codesign -dv --verbose=4` 的输出（stdout/stderr 合并文本）解析签名身份。
pub fn parse_signing_identity(output: &str) -> Option<String> {
    for line in output.lines() {
        if let Some(authority) = line.strip_prefix("Authority=") {
            let authority = authority.trim();
            return (!authority.is_empty()).then(|| authority.to_owned());
        }
        if line.trim() == "Signature=adhoc" {
            return Some("adhoc".to_owned());
        }
    }
    None
}

/// 生产实现：直接参数化调用系统 `codesign`（不经过 shell）。
pub struct MacCodesign;

impl CodeSignVerifier for MacCodesign {
    fn verify_bundle(&mut self, app: &Path) -> Result<(), AppError> {
        let output = std::process::Command::new("/usr/bin/codesign")
            .args(codesign_verify_args(app)?)
            .output()
            .map_err(|error| AppError::new(ErrorCode::PlatformFailed, error.to_string()))?;
        if output.status.success() {
            return Ok(());
        }
        Err(AppError::new(
            ErrorCode::ValidationFailed,
            format!(
                "codesign verification failed for {}: {}",
                app.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ))
    }

    fn signing_identity(&mut self, app: &Path) -> Result<String, AppError> {
        // codesign -d 把元数据写到 stderr，这里合并 stdout+stderr 解析。
        let output = std::process::Command::new("/usr/bin/codesign")
            .args(codesign_identity_args(app)?)
            .output()
            .map_err(|error| AppError::new(ErrorCode::PlatformFailed, error.to_string()))?;
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        if !output.status.success() {
            return Err(AppError::new(
                ErrorCode::ValidationFailed,
                format!(
                    "cannot read the code signature of {}: {}",
                    app.display(),
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }
        parse_signing_identity(&text).ok_or_else(|| {
            AppError::new(
                ErrorCode::ValidationFailed,
                format!("{} has no readable signing identity", app.display()),
            )
        })
    }
}

/// 更新事务完成后的结果。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppUpdateOutcome {
    pub version: String,
    pub restart_required: bool,
}

/// 应用更新事务：下载 → 摘要校验 → 解压 → 结构/版本校验 → 签名深度校验与
/// 同源身份比较 → 备份当前 .app → 原子替换。任何失败不破坏现有安装。
///
/// - `current_bundle` 必须是正在运行的 `.app` 根（非 .app 运行由调用方提前拒绝）；
/// - `staging_dir` 是数据目录下的工作区（下载、解压、previous 备份都在其中）；
/// - `runner` 只在目标父目录不可写时用于 osascript 提权替换。
pub fn update_application_bundle<F, A, S, R>(
    fetcher: &mut F,
    archiver: &mut A,
    signer: &mut S,
    runner: &mut R,
    current_bundle: &Path,
    staging_dir: &Path,
    uid: u32,
) -> Result<AppUpdateOutcome, AppError>
where
    F: ArtifactFetcher,
    A: BundleArchiver,
    S: CodeSignVerifier,
    R: CommandRunner,
{
    let current_version = SemanticVersion::parse(&bundle_short_version(current_bundle)?)?;
    let release = parse_latest_release(&fetcher.fetch(APP_RELEASE_API_URL)?)?;
    if release.version <= current_version {
        return Err(AppError::new(
            ErrorCode::Conflict,
            format!(
                "the installed version {current_version} is already up to date (latest: {})",
                release.version
            ),
        ));
    }
    let digest_text = fetcher.fetch(&release.sha256_url)?;
    let expected_sha256 = parse_sha256_sidecar(&String::from_utf8_lossy(&digest_text))?;

    // 下载与摘要校验（先写文件再校验，失败时目标 .app 尚未被触碰）。
    let download_dir = staging_dir.join("download");
    let extracted_dir = staging_dir.join("staged");
    let archive = download_dir.join(APP_ASSET_NAME);
    fs::create_dir_all(&download_dir).map_err(storage_error)?;
    let bytes = fetcher.fetch(&release.asset_url)?;
    fs::write(&archive, bytes).map_err(storage_error)?;
    let actual_sha256 = format!(
        "{:x}",
        Sha256::digest(&fs::read(&archive).map_err(storage_error)?)
    );
    if actual_sha256 != expected_sha256 {
        return Err(AppError::new(
            ErrorCode::ValidationFailed,
            format!(
                "update archive digest mismatch: expected {expected_sha256}, got {actual_sha256}"
            ),
        ));
    }

    // 解压到 staged，并校验 bundle 结构与内嵌版本。
    archiver.remove_tree(&extracted_dir)?;
    archiver.extract_zip(&archive, &extracted_dir)?;
    let staged_app = extracted_dir.join(APP_BUNDLE_NAME);
    validate_staged_bundle(&staged_app, &release.version)?;

    // 签名：深度校验候选 + 同源身份比较（ad-hoc 只替换 ad-hoc，正式签名同理）。
    signer.verify_bundle(&staged_app)?;
    let staged_identity = signer.signing_identity(&staged_app)?;
    let current_identity = signer.signing_identity(current_bundle)?;
    if staged_identity != current_identity {
        return Err(AppError::new(
            ErrorCode::ValidationFailed,
            format!(
                "signing identity mismatch: update is signed as '{staged_identity}', current installation as '{current_identity}'"
            ),
        ));
    }

    // 备份当前 .app 到数据目录 previous/（人工恢复兜底）。
    let previous_dir = staging_dir.join("previous");
    archiver.remove_tree(&previous_dir)?;
    archiver.copy_tree(current_bundle, &previous_dir.join(APP_BUNDLE_NAME))?;

    // 原子替换：可写直接 rename，不可写走管理员授权脚本（脚本自带回滚）。
    swap_bundle(archiver, runner, current_bundle, &staged_app, uid)?;

    // 清理下载与解压目录；previous/ 保留。
    archiver.remove_tree(&download_dir)?;
    archiver.remove_tree(&extracted_dir)?;
    Ok(AppUpdateOutcome {
        version: release.version.to_string(),
        restart_required: true,
    })
}

/// 校验解压出的 bundle：必须含主可执行，且 Info.plist 版本与 release 一致。
fn validate_staged_bundle(app: &Path, expected: &SemanticVersion) -> Result<(), AppError> {
    let executable = app.join("Contents/MacOS").join(APP_EXECUTABLE_NAME);
    if !executable.is_file() {
        return Err(AppError::new(
            ErrorCode::ValidationFailed,
            format!(
                "the update bundle is missing its executable: {}",
                executable.display()
            ),
        ));
    }
    let bundled = SemanticVersion::parse(&bundle_short_version(app)?)?;
    if bundled != *expected {
        return Err(AppError::new(
            ErrorCode::ValidationFailed,
            format!(
                "update bundle version {bundled} does not match the release version {expected}"
            ),
        ));
    }
    Ok(())
}

/// 替换当前 .app：父目录可写走同卷 rename，否则走 osascript 提权脚本。
fn swap_bundle<A: BundleArchiver, R: CommandRunner>(
    archiver: &mut A,
    runner: &mut R,
    target: &Path,
    staged: &Path,
    uid: u32,
) -> Result<(), AppError> {
    let parent = target.parent().ok_or_else(|| {
        AppError::new(
            ErrorCode::InvalidInput,
            "app bundle path has no parent directory",
        )
    })?;
    if directory_writable(parent) {
        swap_bundle_direct(archiver, target, staged)
    } else {
        swap_bundle_privileged(runner, target, staged, uid)
    }
}

/// 直接替换（目标父目录可写）：候选先拷贝到同卷临时位置，再两次 rename
/// 完成原子替换；最后一步失败时把旧版本改名还原。
fn swap_bundle_direct<A: BundleArchiver>(
    archiver: &mut A,
    target: &Path,
    staged: &Path,
) -> Result<(), AppError> {
    let parent = target.parent().expect("checked by caller");
    let sibling_staged = parent.join(".verge-update-staged.app");
    let previous = parent.join(".verge-update-old.app");
    archiver.remove_tree(&sibling_staged)?;
    archiver.remove_tree(&previous)?;
    archiver.copy_tree(staged, &sibling_staged)?;
    fs::rename(target, &previous).map_err(platform_error)?;
    if let Err(error) = fs::rename(&sibling_staged, target) {
        let rollback = fs::rename(&previous, target).err();
        return Err(match rollback {
            Some(rollback) => AppError::new(
                ErrorCode::PlatformFailed,
                format!("app replacement failed: {error}; rollback failed: {rollback}"),
            ),
            None => platform_error(error),
        });
    }
    archiver.remove_tree(&previous)
}

/// 提权替换（/Applications 等不可写位置）：单行脚本经管理员授权执行，
/// 链式 `&&` 任一失败且目标缺失时自动把旧版本 `mv` 回去，保证不破坏现有安装。
fn swap_bundle_privileged<R: CommandRunner>(
    runner: &mut R,
    target: &Path,
    staged: &Path,
    uid: u32,
) -> Result<(), AppError> {
    let parent = target.parent().expect("checked by caller");
    let previous = parent.join(".verge-update-old.app");
    let target_q = shell_quote(target)?;
    let previous_q = shell_quote(&previous)?;
    let staged_q = shell_quote(staged)?;
    let script = [
        format!("/bin/mv {target_q} {previous_q}"),
        format!("/usr/bin/ditto {staged_q} {target_q}"),
        format!("/usr/sbin/chown -R {uid} {target_q}"),
        format!("/bin/rm -rf {previous_q}"),
    ]
    .join(" && ");
    let script = format!(
        "{script}; rc=$?; if [ $rc -ne 0 ] && [ ! -e {target_q} ] && [ -e {previous_q} ]; then /bin/mv {previous_q} {target_q}; fi; exit $rc"
    );
    run_privileged_script(runner, &script)
}

/// 命令/脚本参数：绝对路径、UTF-8、无控制字符与单引号（与 shell_quote 同约束）。
fn path_arg(path: &Path) -> Result<String, AppError> {
    let value = path
        .to_str()
        .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "path is not valid UTF-8"))?;
    if !path.is_absolute() || value.chars().any(|c| c.is_control() || c == '\'') {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!("path '{value}' is not safe as a command argument"),
        ));
    }
    Ok(value.to_owned())
}

fn storage_error(error: impl fmt::Display) -> AppError {
    AppError::new(ErrorCode::StorageFailed, error.to_string())
}

fn platform_error(error: impl fmt::Display) -> AppError {
    AppError::new(ErrorCode::PlatformFailed, error.to_string())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "verge-app-update-{name}-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    /// 造一个最小可用的 .app：Info.plist 带版本 + 主可执行文件。
    fn write_bundle(dir: &Path, name: &str, version: &str, marker: &str) -> PathBuf {
        let bundle = dir.join(name);
        fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
        fs::write(
            bundle.join("Contents/Info.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\">\n<dict>\n    <key>CFBundleExecutable</key>\n    <string>{APP_EXECUTABLE_NAME}</string>\n    <key>CFBundleShortVersionString</key>\n    <string>{version}</string>\n</dict>\n</plist>\n"
            ),
        )
        .unwrap();
        fs::write(
            bundle.join("Contents/MacOS").join(APP_EXECUTABLE_NAME),
            marker,
        )
        .unwrap();
        bundle
    }

    fn release_json(tag: &str, draft: bool, prerelease: bool) -> Vec<u8> {
        serde_json::json!({
            "tag_name": tag,
            "draft": draft,
            "prerelease": prerelease,
            "assets": [
                {
                    "name": APP_ASSET_NAME,
                    "browser_download_url": format!("{RELEASE_DOWNLOAD_PREFIX}{tag}/{APP_ASSET_NAME}"),
                },
                {
                    "name": format!("{APP_ASSET_NAME}.sha256"),
                    "browser_download_url": format!("{RELEASE_DOWNLOAD_PREFIX}{tag}/{APP_ASSET_NAME}.sha256"),
                },
                {
                    "name": "Verge-macos-x86_64.zip",
                    "browser_download_url": format!("{RELEASE_DOWNLOAD_PREFIX}{tag}/Verge-macos-x86_64.zip"),
                }
            ]
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn semantic_version_parses_and_orders() {
        assert_eq!(
            SemanticVersion::parse("v1.2.3").unwrap(),
            SemanticVersion {
                major: 1,
                minor: 2,
                patch: 3
            }
        );
        assert!(SemanticVersion::parse("0.2.0").unwrap() > SemanticVersion::parse("0.1").unwrap());
        assert_eq!(
            SemanticVersion::parse("0.2").unwrap(),
            SemanticVersion::parse("0.2.0").unwrap()
        );
        assert!(
            SemanticVersion::parse("v0.10.0").unwrap() > SemanticVersion::parse("v0.9.9").unwrap()
        );
        assert_eq!(
            SemanticVersion::parse("1.0.0").unwrap(),
            SemanticVersion::parse("v1.0.0").unwrap()
        );
        for bad in ["", "1", "1.2.3.4", "1.2-beta", "v1.x.0", "1..2"] {
            assert_eq!(
                SemanticVersion::parse(bad).unwrap_err().code,
                ErrorCode::ValidationFailed,
                "should reject {bad}"
            );
        }
    }

    #[test]
    fn latest_release_selects_arm64_asset_and_its_sidecar() {
        let release = parse_latest_release(&release_json("v0.2.0", false, false)).unwrap();
        assert_eq!(release.version.to_string(), "0.2.0");
        assert!(release.asset_url.ends_with(APP_ASSET_NAME));
        assert!(
            release
                .sha256_url
                .ends_with(&format!("{APP_ASSET_NAME}.sha256"))
        );
    }

    #[test]
    fn drafts_prereleases_missing_assets_and_foreign_urls_are_rejected() {
        for json in [
            release_json("v0.2.0", true, false),
            release_json("v0.2.0", false, true),
        ] {
            assert_eq!(
                parse_latest_release(&json).unwrap_err().code,
                ErrorCode::NotFound
            );
        }
        let no_assets = serde_json::json!({
            "tag_name": "v0.2.0",
            "assets": []
        })
        .to_string();
        assert_eq!(
            parse_latest_release(no_assets.as_bytes()).unwrap_err().code,
            ErrorCode::NotFound
        );
        let foreign = serde_json::json!({
            "tag_name": "v0.2.0",
            "assets": [
                { "name": APP_ASSET_NAME, "browser_download_url": "https://evil.example.com/x.zip" },
                { "name": format!("{APP_ASSET_NAME}.sha256"), "browser_download_url": format!("{RELEASE_DOWNLOAD_PREFIX}v0.2.0/{APP_ASSET_NAME}.sha256") },
            ]
        })
        .to_string();
        assert_eq!(
            parse_latest_release(foreign.as_bytes()).unwrap_err().code,
            ErrorCode::ValidationFailed
        );
    }

    #[test]
    fn sha256_sidecar_accepts_shasum_output_and_rejects_garbage() {
        let digest = "a".repeat(64);
        assert_eq!(
            parse_sha256_sidecar(&format!(
                "{}  Verge-macos-arm64.zip\n",
                digest.to_uppercase()
            ))
            .unwrap(),
            digest
        );
        for bad in ["", "abc  file.zip", &"g".repeat(64)] {
            assert_eq!(
                parse_sha256_sidecar(bad).unwrap_err().code,
                ErrorCode::ValidationFailed
            );
        }
    }

    struct FakeArtifactFetcher {
        responses: VecDeque<Result<Vec<u8>, AppError>>,
        requested_urls: Vec<String>,
    }

    impl ArtifactFetcher for FakeArtifactFetcher {
        fn fetch(&mut self, url: &str) -> Result<Vec<u8>, AppError> {
            self.requested_urls.push(url.to_owned());
            self.responses.pop_front().expect("unexpected fetch")
        }
    }

    #[test]
    fn check_app_update_compares_versions() {
        let mut fetcher = FakeArtifactFetcher {
            responses: VecDeque::from([Ok(release_json("v0.2.0", false, false))]),
            requested_urls: Vec::new(),
        };
        let status = check_app_update(&mut fetcher, Some("0.1.0")).unwrap();
        assert_eq!(status.current_version.as_deref(), Some("0.1.0"));
        assert_eq!(status.latest_version, "0.2.0");
        assert!(status.update_available);
        assert_eq!(fetcher.requested_urls, [APP_RELEASE_API_URL]);

        let mut fetcher = FakeArtifactFetcher {
            responses: VecDeque::from([Ok(release_json("v0.2.0", false, false))]),
            requested_urls: Vec::new(),
        };
        let status = check_app_update(&mut fetcher, Some("0.2.0")).unwrap();
        assert!(!status.update_available);

        // 非 .app 运行：没有当前版本，返回最新版但不提示可更新。
        let mut fetcher = FakeArtifactFetcher {
            responses: VecDeque::from([Ok(release_json("v0.2.0", false, false))]),
            requested_urls: Vec::new(),
        };
        let status = check_app_update(&mut fetcher, None).unwrap();
        assert_eq!(status.current_version, None);
        assert!(!status.update_available);
    }

    /// fake 解压：从预置的“zip 内容”目录拷贝到目标（extract_source 扮演 zip）。
    struct FakeArchiver {
        extract_source: PathBuf,
        calls: Vec<String>,
    }

    impl FakeArchiver {
        fn copy_recursive(source: &Path, destination: &Path) {
            fs::create_dir_all(destination).unwrap();
            for entry in fs::read_dir(source).unwrap() {
                let entry = entry.unwrap();
                let target = destination.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    Self::copy_recursive(&entry.path(), &target);
                } else {
                    fs::copy(entry.path(), &target).unwrap();
                }
            }
        }
    }

    impl BundleArchiver for FakeArchiver {
        fn extract_zip(&mut self, _archive: &Path, destination: &Path) -> Result<(), AppError> {
            self.calls
                .push(format!("extract:{}", destination.display()));
            Self::copy_recursive(&self.extract_source, destination);
            Ok(())
        }

        fn copy_tree(&mut self, source: &Path, destination: &Path) -> Result<(), AppError> {
            self.calls.push(format!("copy:{}", destination.display()));
            Self::copy_recursive(source, destination);
            Ok(())
        }

        fn remove_tree(&mut self, path: &Path) -> Result<(), AppError> {
            self.calls.push(format!("remove:{}", path.display()));
            if path.exists() {
                fs::remove_dir_all(path).unwrap();
            }
            Ok(())
        }
    }

    struct FakeSigner {
        /// bundle 主可执行内容 → 签名身份（None = 签名校验失败）。
        identities: Vec<(String, String)>,
        verified: Vec<PathBuf>,
    }

    impl CodeSignVerifier for FakeSigner {
        fn verify_bundle(&mut self, app: &Path) -> Result<(), AppError> {
            self.verified.push(app.to_owned());
            let marker = fs::read(app.join("Contents/MacOS").join(APP_EXECUTABLE_NAME))
                .map_err(|error| platform_error(error.to_string()))?;
            let marker = String::from_utf8_lossy(&marker).into_owned();
            if self.identities.iter().any(|(m, _)| *m == marker) {
                Ok(())
            } else {
                Err(AppError::new(
                    ErrorCode::ValidationFailed,
                    "invalid signature",
                ))
            }
        }

        fn signing_identity(&mut self, app: &Path) -> Result<String, AppError> {
            let marker = fs::read(app.join("Contents/MacOS").join(APP_EXECUTABLE_NAME))
                .map_err(|error| platform_error(error.to_string()))?;
            let marker = String::from_utf8_lossy(&marker).into_owned();
            self.identities
                .iter()
                .find(|(m, _)| *m == marker)
                .map(|(_, identity)| identity.clone())
                .ok_or_else(|| AppError::new(ErrorCode::ValidationFailed, "unsigned"))
        }
    }

    #[derive(Default)]
    struct FakeRunner {
        calls: Vec<(String, Vec<String>)>,
        outputs: VecDeque<Result<String, AppError>>,
    }

    impl CommandRunner for FakeRunner {
        fn run(&mut self, program: &str, args: &[String]) -> Result<String, AppError> {
            self.calls.push((program.to_owned(), args.to_vec()));
            self.outputs
                .pop_front()
                .unwrap_or_else(|| Ok(String::new()))
        }
    }

    struct UpdateFixture {
        _dir: TestDir,
        install_dir: PathBuf,
        current_bundle: PathBuf,
        staging: PathBuf,
        fetcher: FakeArtifactFetcher,
        archiver: FakeArchiver,
        signer: FakeSigner,
        runner: FakeRunner,
    }

    /// 组装一次成功更新所需的全部 fake：当前 .app v0.1.0，release v0.2.0。
    fn update_fixture(name: &str) -> UpdateFixture {
        let dir = TestDir::new(name);
        let install_dir = dir.0.join("install");
        fs::create_dir_all(&install_dir).unwrap();
        let current_bundle = write_bundle(&install_dir, APP_BUNDLE_NAME, "0.1.0", "old-binary");

        // “zip 内容”：fake 解压器从这个目录拷贝出 v0.2.0 的新 bundle。
        let zip_content = dir.0.join("zip-content");
        write_bundle(&zip_content, APP_BUNDLE_NAME, "0.2.0", "new-binary");
        let zip_bytes = b"fake zip bytes".to_vec();
        let digest = sha256_hex(&zip_bytes);

        let fetcher = FakeArtifactFetcher {
            responses: VecDeque::from([
                Ok(release_json("v0.2.0", false, false)),
                Ok(format!("{digest}  {APP_ASSET_NAME}\n").into_bytes()),
                Ok(zip_bytes.clone()),
            ]),
            requested_urls: Vec::new(),
        };
        let signer = FakeSigner {
            identities: vec![
                ("old-binary".into(), "adhoc".into()),
                ("new-binary".into(), "adhoc".into()),
            ],
            verified: Vec::new(),
        };
        UpdateFixture {
            install_dir,
            current_bundle,
            staging: dir.0.join("staging"),
            fetcher,
            archiver: FakeArchiver {
                extract_source: zip_content,
                calls: Vec::new(),
            },
            signer,
            runner: FakeRunner::default(),
            _dir: dir,
        }
    }

    #[test]
    fn update_replaces_bundle_and_keeps_previous_backup() {
        let mut fixture = update_fixture("success");
        let outcome = update_application_bundle(
            &mut fixture.fetcher,
            &mut fixture.archiver,
            &mut fixture.signer,
            &mut fixture.runner,
            &fixture.current_bundle,
            &fixture.staging,
            501,
        )
        .unwrap();

        assert_eq!(outcome.version, "0.2.0");
        assert!(outcome.restart_required);
        // 目标已被新版替换，且深签名校验发生在替换之前。
        assert_eq!(
            fs::read(
                fixture
                    .current_bundle
                    .join("Contents/MacOS")
                    .join(APP_EXECUTABLE_NAME)
            )
            .unwrap(),
            b"new-binary"
        );
        assert_eq!(fixture.signer.verified.len(), 1);
        // previous 备份保留旧版，可人工还原。
        assert_eq!(
            fs::read(
                fixture
                    .staging
                    .join("previous")
                    .join(APP_BUNDLE_NAME)
                    .join("Contents/MacOS")
                    .join(APP_EXECUTABLE_NAME)
            )
            .unwrap(),
            b"old-binary"
        );
        // 可写目录走直接 rename，不触发提权；同卷临时目录已清理。
        assert!(fixture.runner.calls.is_empty());
        assert!(
            !fixture
                .install_dir
                .join(".verge-update-staged.app")
                .exists()
        );
        assert!(!fixture.install_dir.join(".verge-update-old.app").exists());
        assert!(!fixture.staging.join("download").exists());
        assert!(!fixture.staging.join("staged").exists());
    }

    #[test]
    fn digest_mismatch_rejects_before_touching_the_installation() {
        let mut fixture = update_fixture("digest");
        fixture.fetcher.responses[1] = Ok("0".repeat(64).into_bytes());

        let error = update_application_bundle(
            &mut fixture.fetcher,
            &mut fixture.archiver,
            &mut fixture.signer,
            &mut fixture.runner,
            &fixture.current_bundle,
            &fixture.staging,
            501,
        )
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::ValidationFailed);
        assert!(error.message.contains("digest mismatch"));
        // 未解压、未验签、未替换。
        assert!(fixture.signer.verified.is_empty());
        assert_eq!(
            fs::read(
                fixture
                    .current_bundle
                    .join("Contents/MacOS")
                    .join(APP_EXECUTABLE_NAME)
            )
            .unwrap(),
            b"old-binary"
        );
    }

    #[test]
    fn bundle_version_mismatch_is_rejected() {
        let mut fixture = update_fixture("bundle-version");
        // “zip 内容”里的 plist 版本与 release tag 不一致。
        write_bundle(
            &fixture.archiver.extract_source,
            APP_BUNDLE_NAME,
            "0.9.9",
            "new-binary",
        );

        let error = update_application_bundle(
            &mut fixture.fetcher,
            &mut fixture.archiver,
            &mut fixture.signer,
            &mut fixture.runner,
            &fixture.current_bundle,
            &fixture.staging,
            501,
        )
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::ValidationFailed);
        assert!(error.message.contains("does not match the release version"));
        assert!(fixture.signer.verified.is_empty());
    }

    #[test]
    fn foreign_signing_identity_is_rejected_before_replacement() {
        let mut fixture = update_fixture("identity");
        fixture.signer.identities[1] = (
            "new-binary".into(),
            "Developer ID Application: Someone Else (TEAM)".into(),
        );

        let error = update_application_bundle(
            &mut fixture.fetcher,
            &mut fixture.archiver,
            &mut fixture.signer,
            &mut fixture.runner,
            &fixture.current_bundle,
            &fixture.staging,
            501,
        )
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::ValidationFailed);
        assert!(error.message.contains("signing identity mismatch"));
        // 深度校验确实执行过，但替换没有发生。
        assert_eq!(fixture.signer.verified.len(), 1);
        assert_eq!(
            fs::read(
                fixture
                    .current_bundle
                    .join("Contents/MacOS")
                    .join(APP_EXECUTABLE_NAME)
            )
            .unwrap(),
            b"old-binary"
        );
    }

    #[test]
    fn invalid_candidate_signature_is_rejected() {
        let mut fixture = update_fixture("bad-signature");
        fixture.signer.identities.remove(1);

        let error = update_application_bundle(
            &mut fixture.fetcher,
            &mut fixture.archiver,
            &mut fixture.signer,
            &mut fixture.runner,
            &fixture.current_bundle,
            &fixture.staging,
            501,
        )
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::ValidationFailed);
        assert_eq!(
            fs::read(
                fixture
                    .current_bundle
                    .join("Contents/MacOS")
                    .join(APP_EXECUTABLE_NAME)
            )
            .unwrap(),
            b"old-binary"
        );
    }

    #[test]
    fn up_to_date_installation_is_a_conflict_without_side_effects() {
        let mut fixture = update_fixture("up-to-date");
        fixture.fetcher.responses[0] = Ok(release_json("v0.1.0", false, false));

        let error = update_application_bundle(
            &mut fixture.fetcher,
            &mut fixture.archiver,
            &mut fixture.signer,
            &mut fixture.runner,
            &fixture.current_bundle,
            &fixture.staging,
            501,
        )
        .unwrap_err();

        assert_eq!(error.code, ErrorCode::Conflict);
        // 只拉了 release JSON，没有下载资产。
        assert_eq!(fixture.fetcher.requested_urls.len(), 1);
    }

    #[test]
    fn unwritable_parent_uses_a_privileged_script_with_builtin_rollback() {
        let fixture_dir = TestDir::new("privileged");
        let install_dir = fixture_dir.0.join("install");
        fs::create_dir_all(&install_dir).unwrap();
        let target = write_bundle(&install_dir, APP_BUNDLE_NAME, "0.1.0", "old-binary");
        let staged_dir = fixture_dir.0.join("staged");
        let staged = write_bundle(&staged_dir, APP_BUNDLE_NAME, "0.2.0", "new-binary");

        // 直接调用提权路径，锁定脚本形态（“父目录不可写”在测试环境不可便携模拟，
        // 分发条件 directory_writable 由 verge-platform 的 access(2) 单测覆盖）。
        let mut runner = FakeRunner::default();
        swap_bundle_privileged(&mut runner, &target, &staged, 501).unwrap();
        assert_eq!(runner.calls.len(), 1);
        let (program, args) = &runner.calls[0];
        assert_eq!(program, "/usr/bin/osascript");
        let script = &args[1];
        assert!(script.starts_with("do shell script \""));
        assert!(script.ends_with("\" with administrator privileges"));
        for expected in [
            format!(
                "/bin/mv '{}' '{}'",
                target.display(),
                install_dir.join(".verge-update-old.app").display()
            ),
            format!(
                "/usr/bin/ditto '{}' '{}'",
                staged.display(),
                target.display()
            ),
            format!("/usr/sbin/chown -R 501 '{}'", target.display()),
            format!(
                "/bin/rm -rf '{}'",
                install_dir.join(".verge-update-old.app").display()
            ),
            "rc=$?".to_owned(),
            "exit $rc".to_owned(),
        ] {
            assert!(script.contains(&expected), "script missing: {expected}");
        }

        // 提权脚本失败：错误可读，目标未被 fake runner 触碰（真实系统由脚本自回滚）。
        let mut runner = FakeRunner {
            calls: Vec::new(),
            outputs: VecDeque::from([Err(AppError::new(
                ErrorCode::PlatformFailed,
                "osascript: User canceled",
            ))]),
        };
        let error = swap_bundle_privileged(&mut runner, &target, &staged, 501).unwrap_err();
        assert!(error.message.contains("User canceled"));
        assert_eq!(
            fs::read(target.join("Contents/MacOS").join(APP_EXECUTABLE_NAME)).unwrap(),
            b"old-binary"
        );
    }

    #[test]
    fn codesign_command_assembly_and_identity_parsing() {
        let app = Path::new("/Applications/Verge.app");
        assert_eq!(
            codesign_verify_args(app).unwrap(),
            [
                "--verify".to_owned(),
                "--deep".to_owned(),
                "--strict".to_owned(),
                "/Applications/Verge.app".to_owned()
            ]
        );
        assert_eq!(
            codesign_identity_args(app).unwrap(),
            [
                "-dv".to_owned(),
                "--verbose=4".to_owned(),
                "/Applications/Verge.app".to_owned()
            ]
        );
        // 含单引号或控制字符的路径被拒绝而不是转义。
        assert!(codesign_verify_args(Path::new("/tmp/evil'app")).is_err());

        let signed = "Executable=/Applications/Verge.app/Contents/MacOS/verge-gpui\nAuthority=Developer ID Application: Example (TEAMID)\nAuthority=Developer ID Certification Authority\n";
        assert_eq!(
            parse_signing_identity(signed).as_deref(),
            Some("Developer ID Application: Example (TEAMID)")
        );
        assert_eq!(
            parse_signing_identity("Signature=adhoc\nInfo.plist=not bound").as_deref(),
            Some("adhoc")
        );
        assert_eq!(parse_signing_identity("code object is not signed"), None);
    }

    /// 直接替换：预置的同名残留先被清理，替换后目标为新版且不留临时目录。
    #[test]
    fn direct_swap_replaces_and_cleans_up_siblings() {
        let dir = TestDir::new("direct-swap");
        let install_dir = dir.0.join("install");
        fs::create_dir_all(&install_dir).unwrap();
        let target = write_bundle(&install_dir, APP_BUNDLE_NAME, "0.1.0", "old-binary");
        let staged_root = dir.0.join("staged");
        let staged = write_bundle(&staged_root, APP_BUNDLE_NAME, "0.2.0", "new-binary");
        // 预置同卷残留，验证先清理。
        fs::create_dir_all(install_dir.join(".verge-update-old.app")).unwrap();

        let mut archiver = FakeArchiver {
            extract_source: PathBuf::new(),
            calls: Vec::new(),
        };
        swap_bundle_direct(&mut archiver, &target, &staged).unwrap();
        assert_eq!(
            fs::read(target.join("Contents/MacOS").join(APP_EXECUTABLE_NAME)).unwrap(),
            b"new-binary"
        );
        assert!(!install_dir.join(".verge-update-old.app").exists());
        assert!(!install_dir.join(".verge-update-staged.app").exists());
    }
}
