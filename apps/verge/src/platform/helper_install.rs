//! 特权 helper 的安装/修复/卸载。
//!
//! 约束：
//! - 提权只通过 `/usr/bin/osascript -e 'do shell script "..." with administrator privileges'`；
//! - 脚本由固定命令模板 + 校验过的绝对路径参数组成，不拼接任何用户输入；
//! - 安装前校验随包 helper 二进制的 SHA-256 与构建时记录值一致；
//! - 多步失败时回滚已完成步骤（等价于卸载），错误信息可读；
//! - 全部系统交互经 `CommandRunner` 注入，测试用 fake，不触达真实系统。

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::domain::{AppError, ErrorCode};
use sha2::{Digest, Sha256};

use super::{CommandRunner, OSASCRIPT};

pub const HELPER_LABEL: &str = "com.zzzgydi.verge.helper";
pub const HELPER_SOCKET_PATH: &str = "/var/run/verge-helper.sock";
const HELPER_BINARY_NAME: &str = "verge-helper";

/// 安装落点布局。生产环境用 `system()`；测试注入临时目录。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HelperInstallLayout {
    /// /Library/PrivilegedHelperTools
    pub install_dir: PathBuf,
    /// /Library/LaunchDaemons
    pub daemons_dir: PathBuf,
}

impl HelperInstallLayout {
    pub fn system() -> Self {
        Self {
            install_dir: PathBuf::from("/Library/PrivilegedHelperTools"),
            daemons_dir: PathBuf::from("/Library/LaunchDaemons"),
        }
    }

    pub fn helper_binary(&self) -> PathBuf {
        self.install_dir.join(HELPER_BINARY_NAME)
    }

    pub fn daemon_plist(&self) -> PathBuf {
        self.daemons_dir.join(format!("{HELPER_LABEL}.plist"))
    }
}

/// 随包 helper 二进制 + 构建时记录的预期 SHA-256。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HelperBundle {
    pub binary: PathBuf,
    pub expected_sha256: String,
}

/// 当前进程若位于 `.app` 内，返回其 Resources 目录；否则 None。
pub fn bundled_resources_directory() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents")
        .then(|| contents.join("Resources"))
}

/// 在 `.app` Resources 下定位 helper 二进制与构建期摘要文件
/// （布局见 apps/verge/scripts/build-macos-app.sh）。
pub fn discover_bundled_helper(resources: &Path) -> Result<HelperBundle, AppError> {
    let binary = resources.join("helper").join(HELPER_BINARY_NAME);
    if !binary.is_file() {
        return Err(AppError::new(
            ErrorCode::PlatformFailed,
            format!(
                "bundled helper not found at {}; package the app with apps/verge/scripts/build-macos-app.sh first",
                binary.display()
            ),
        ));
    }
    let digest_path = resources.join("helper").join("verge-helper.sha256");
    let expected = fs::read_to_string(&digest_path)
        .map_err(|error| {
            AppError::new(
                ErrorCode::PlatformFailed,
                format!(
                    "bundled helper digest missing at {}: {error}",
                    digest_path.display()
                ),
            )
        })?
        .trim()
        .to_owned();
    Ok(HelperBundle {
        binary,
        expected_sha256: expected,
    })
}

/// 允许连接 helper 的 UID 取当前真实用户。
pub fn current_uid() -> u32 {
    // SAFETY: getuid has no preconditions.
    unsafe { libc::getuid() }
}

/// 安装器：全部特权动作经注入的 runner 执行（osascript 提权）。
pub struct MacHelperInstaller<R> {
    runner: R,
    layout: HelperInstallLayout,
    staging_dir: PathBuf,
    uid: u32,
}

impl<R: CommandRunner> MacHelperInstaller<R> {
    pub fn new(
        runner: R,
        layout: HelperInstallLayout,
        staging_dir: PathBuf,
        uid: u32,
    ) -> Result<Self, AppError> {
        for (name, path) in [
            ("install dir", &layout.install_dir),
            ("launch daemons dir", &layout.daemons_dir),
            ("staging dir", &staging_dir),
        ] {
            if !path.is_absolute() {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    format!("helper {name} must be an absolute path"),
                ));
            }
        }
        Ok(Self {
            runner,
            layout,
            staging_dir,
            uid,
        })
    }

    /// 安装/修复：校验来源摘要 → 清理残留（bootout + 删旧文件，尽力而为）
    /// → 提权安装（拷二进制、写 LaunchDaemon plist、bootstrap + kickstart）。
    /// 安装脚本中途失败时回滚已完成的步骤。
    pub fn install(&mut self, bundle: &HelperBundle) -> Result<(), AppError> {
        verify_bundle_source(bundle)?;
        let staged_plist = self.stage_daemon_plist()?;
        let cleanup = uninstall_script(&self.layout)?;
        // 残留清理失败不阻塞安装（例如从未安装过时 bootout 必然失败）。
        let _ = self.run_privileged(&cleanup);
        let install = install_script(&self.layout, &bundle.binary, &staged_plist)?;
        if let Err(cause) = self.run_privileged(&install) {
            return Err(match self.run_privileged(&cleanup) {
                Ok(()) => AppError::new(
                    ErrorCode::PlatformFailed,
                    format!("helper install failed: {cause}; completed steps were rolled back"),
                ),
                Err(rollback) => AppError::new(
                    ErrorCode::PlatformFailed,
                    format!("helper install failed: {cause}; rollback failed: {rollback}"),
                ),
            });
        }
        Ok(())
    }

    /// 卸载：bootout LaunchDaemon、删除 plist 与二进制（同一条提权路径）。
    pub fn uninstall(&mut self) -> Result<(), AppError> {
        self.run_privileged(&uninstall_script(&self.layout)?)
    }

    /// 写 LaunchDaemon plist 到暂存目录（非特权），供提权脚本拷贝。
    fn stage_daemon_plist(&self) -> Result<PathBuf, AppError> {
        fs::create_dir_all(&self.staging_dir).map_err(storage_error)?;
        let path = self.staging_dir.join(format!("{HELPER_LABEL}.plist"));
        let content = daemon_plist(
            &self.layout.helper_binary(),
            Path::new(HELPER_SOCKET_PATH),
            self.uid,
        )?;
        fs::write(&path, content).map_err(storage_error)?;
        Ok(path)
    }

    /// 单条提权执行：委托给自由函数，保持与 helper 安装同一条提权路径。
    fn run_privileged(&mut self, script: &str) -> Result<(), AppError> {
        run_privileged_script(&mut self.runner, script)
    }
}

/// 单条提权执行（osascript 管理员授权）：脚本必须单行，经 AppleScript 字符串
/// 转义后交给 osascript。供 helper 安装与应用自身更新等场景复用同一条路径。
pub fn run_privileged_script<R: CommandRunner>(
    runner: &mut R,
    script: &str,
) -> Result<(), AppError> {
    if script.contains('\n') || script.contains('\r') {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "privileged helper script must be a single line",
        ));
    }
    let escaped = script.replace('\\', "\\\\").replace('"', "\\\"");
    runner
        .run(
            OSASCRIPT,
            &[
                "-e".into(),
                format!("do shell script \"{escaped}\" with administrator privileges"),
            ],
        )
        .map(|_| ())
}

/// 校验来源二进制 SHA-256 与构建时记录值一致；不一致直接拒绝。
fn verify_bundle_source(bundle: &HelperBundle) -> Result<(), AppError> {
    let expected = bundle.expected_sha256.trim().to_ascii_lowercase();
    if expected.len() != 64 || !expected.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AppError::new(
            ErrorCode::ValidationFailed,
            "bundled helper digest is malformed (expected 64 hex characters)",
        ));
    }
    let bytes = fs::read(&bundle.binary).map_err(|error| {
        AppError::new(
            ErrorCode::PlatformFailed,
            format!(
                "cannot read bundled helper {}: {error}",
                bundle.binary.display()
            ),
        )
    })?;
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected {
        return Err(AppError::new(
            ErrorCode::ValidationFailed,
            format!("bundled helper SHA-256 mismatch: expected {expected}, got {actual}"),
        ));
    }
    Ok(())
}

/// 安装脚本：固定命令模板，路径参数经 `shell_quote` 校验后单引号包裹。
fn install_script(
    layout: &HelperInstallLayout,
    source: &Path,
    staged_plist: &Path,
) -> Result<String, AppError> {
    let helper = layout.helper_binary();
    let plist = layout.daemon_plist();
    Ok([
        format!("/bin/mkdir -p {}", shell_quote(&layout.install_dir)?),
        format!(
            "/usr/bin/ditto {} {}",
            shell_quote(source)?,
            shell_quote(&helper)?
        ),
        format!("/usr/sbin/chown root:wheel {}", shell_quote(&helper)?),
        format!("/bin/chmod 755 {}", shell_quote(&helper)?),
        format!(
            "/usr/bin/ditto {} {}",
            shell_quote(staged_plist)?,
            shell_quote(&plist)?
        ),
        format!("/usr/sbin/chown root:wheel {}", shell_quote(&plist)?),
        format!("/bin/chmod 644 {}", shell_quote(&plist)?),
        format!("/bin/launchctl bootstrap system {}", shell_quote(&plist)?),
        format!("/bin/launchctl kickstart -k system/{HELPER_LABEL}"),
    ]
    .join(" && "))
}

/// 卸载/清理脚本：bootout 容忍“未加载”，文件删除幂等。
fn uninstall_script(layout: &HelperInstallLayout) -> Result<String, AppError> {
    Ok(format!(
        "/bin/launchctl bootout system/{HELPER_LABEL} || true; /bin/rm -f {} && /bin/rm -f {}",
        shell_quote(&layout.daemon_plist())?,
        shell_quote(&layout.helper_binary())?,
    ))
}

/// LaunchDaemon plist：固定模板；label/socket/二进制路径来自常量与已校验路径，
/// UID 为数值，均不接受用户输入。
fn daemon_plist(helper: &Path, socket: &Path, uid: u32) -> Result<String, AppError> {
    let helper = plist_value(helper)?;
    let socket = plist_value(socket)?;
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{HELPER_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{helper}</string>
        <string>--socket</string>
        <string>{socket}</string>
        <string>--uid</string>
        <string>{uid}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/var/log/verge-helper.log</string>
    <key>StandardErrorPath</key>
    <string>/var/log/verge-helper.log</string>
</dict>
</plist>
"#
    ))
}

/// plist 字符串值的防御性校验（路径不含 XML 特殊字符与控制字符）。
fn plist_value(path: &Path) -> Result<String, AppError> {
    let value = path
        .to_str()
        .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "helper path is not valid UTF-8"))?;
    if value
        .chars()
        .any(|character| character.is_control() || matches!(character, '<' | '>' | '&'))
    {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!("helper path '{value}' is not safe for the LaunchDaemon plist"),
        ));
    }
    Ok(value.to_owned())
}

/// shell 参数校验 + 单引号包裹：绝对路径、无控制字符、无单引号（有则拒绝而非转义）。
/// 提权脚本的所有路径参数必须经过它（应用更新替换脚本同用）。
pub fn shell_quote(path: &Path) -> Result<String, AppError> {
    let value = path
        .to_str()
        .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "helper path is not valid UTF-8"))?;
    if !path.is_absolute()
        || value
            .chars()
            .any(|character| character.is_control() || character == '\'')
    {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!("helper path '{value}' is not safe for the privileged script"),
        ));
    }
    Ok(format!("'{value}'"))
}

fn storage_error(error: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::StorageFailed, error.to_string())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[derive(Default)]
    struct FakeRunner {
        outputs: VecDeque<Result<String, AppError>>,
        /// 记录每次调用（含失败的），与 outputs 一一对应。
        calls: Vec<Vec<String>>,
    }

    impl CommandRunner for FakeRunner {
        fn run(&mut self, program: &str, args: &[String]) -> Result<String, AppError> {
            let mut call = vec![program.to_owned()];
            call.extend_from_slice(args);
            self.calls.push(call);
            self.outputs
                .pop_front()
                .unwrap_or_else(|| Ok(String::new()))
        }
    }

    struct TestDir(PathBuf);

    static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    impl TestDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "verge-helper-install-{}-{nonce}-{}",
                std::process::id(),
                TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
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

    fn layout_in(dir: &TestDir) -> HelperInstallLayout {
        HelperInstallLayout {
            install_dir: dir.0.join("PrivilegedHelperTools"),
            daemons_dir: dir.0.join("LaunchDaemons"),
        }
    }

    fn bundle_in(dir: &TestDir, bytes: &[u8]) -> HelperBundle {
        let binary = dir.0.join("verge-helper");
        fs::write(&binary, bytes).unwrap();
        HelperBundle {
            binary,
            expected_sha256: format!("{:x}", Sha256::digest(bytes)),
        }
    }

    fn make_installer(runner: FakeRunner, dir: &TestDir) -> MacHelperInstaller<FakeRunner> {
        MacHelperInstaller::new(runner, layout_in(dir), dir.0.join("staging"), 501).unwrap()
    }

    fn privileged_scripts(runner: &FakeRunner) -> Vec<String> {
        runner
            .calls
            .iter()
            .map(|call| {
                assert_eq!(call[0], OSASCRIPT);
                assert_eq!(call[1], "-e");
                call[2].clone()
            })
            .collect()
    }

    #[test]
    fn install_runs_cleanup_then_privileged_install_with_admin_authorization() {
        let dir = TestDir::new();
        let bundle = bundle_in(&dir, b"verge-helper-binary");
        let mut installer = make_installer(FakeRunner::default(), &dir);

        installer.install(&bundle).unwrap();

        let scripts = privileged_scripts(&installer.runner);
        assert_eq!(scripts.len(), 2);
        for script in &scripts {
            assert!(script.starts_with("do shell script \""));
            assert!(script.ends_with("\" with administrator privileges"));
        }
        // 第一步：清理残留（bootout 容忍失败 + 幂等删除）。
        assert!(scripts[0].contains(&format!("bootout system/{HELPER_LABEL}")));
        assert!(scripts[0].contains("|| true;"));
        // 第二步：完整安装序列。
        let install = &scripts[1];
        let helper = layout_in(&dir).helper_binary();
        let plist = layout_in(&dir).daemon_plist();
        for fragment in [
            format!("/bin/mkdir -p '{}'", layout_in(&dir).install_dir.display()),
            format!("ditto '{}' '{}'", bundle.binary.display(), helper.display()),
            format!("/usr/sbin/chown root:wheel '{}'", helper.display()),
            format!("/bin/chmod 755 '{}'", helper.display()),
            format!("/bin/chmod 644 '{}'", plist.display()),
            format!("/bin/launchctl bootstrap system '{}'", plist.display()),
            format!("/bin/launchctl kickstart -k system/{HELPER_LABEL}"),
        ] {
            assert!(
                install.contains(&fragment),
                "install script misses {fragment}"
            );
        }
        // 暂存 plist 内容：固定 label/socket + 当前用户 UID。
        let staged =
            fs::read_to_string(dir.0.join(format!("staging/{HELPER_LABEL}.plist"))).unwrap();
        assert!(staged.contains(&format!("<string>{HELPER_LABEL}</string>")));
        assert!(staged.contains(&format!("<string>{HELPER_SOCKET_PATH}</string>")));
        assert!(staged.contains(&format!("<string>{}</string>", helper.display())));
        assert!(staged.contains("<string>501</string>"));
    }

    #[test]
    fn install_rejects_a_digest_mismatch_before_any_privileged_call() {
        let dir = TestDir::new();
        let mut bundle = bundle_in(&dir, b"verge-helper-binary");
        bundle.expected_sha256 = "0".repeat(64);
        let mut installer = make_installer(FakeRunner::default(), &dir);

        let error = installer.install(&bundle).unwrap_err();

        assert_eq!(error.code, ErrorCode::ValidationFailed);
        assert!(error.message.contains("SHA-256 mismatch"));
        assert!(installer.runner.calls.is_empty());
        // 格式非法的摘要同样拒绝。
        let mut malformed = bundle_in(&dir, b"verge-helper-binary");
        malformed.expected_sha256 = "not-hex".into();
        let mut second = make_installer(FakeRunner::default(), &dir);
        assert_eq!(
            second.install(&malformed).unwrap_err().code,
            ErrorCode::ValidationFailed
        );
        assert!(second.runner.calls.is_empty());
    }

    #[test]
    fn failed_install_rolls_back_completed_steps() {
        let dir = TestDir::new();
        let bundle = bundle_in(&dir, b"verge-helper-binary");
        let mut runner = FakeRunner::default();
        runner.outputs.push_back(Ok(String::new())); // 残留清理
        runner.outputs.push_back(Err(AppError::new(
            ErrorCode::PlatformFailed,
            "osascript: User canceled",
        )));
        runner.outputs.push_back(Ok(String::new())); // 回滚
        let mut installer = make_installer(runner, &dir);

        let error = installer.install(&bundle).unwrap_err();

        assert!(error.message.contains("User canceled"));
        assert!(error.message.contains("rolled back"));
        let scripts = privileged_scripts(&installer.runner);
        assert_eq!(scripts.len(), 3);
        assert!(scripts[2].contains("bootout"));
    }

    #[test]
    fn rollback_failure_is_reported_in_the_error() {
        let dir = TestDir::new();
        let bundle = bundle_in(&dir, b"verge-helper-binary");
        let mut runner = FakeRunner::default();
        runner.outputs.push_back(Ok(String::new()));
        runner.outputs.push_back(Err(AppError::new(
            ErrorCode::PlatformFailed,
            "install broke",
        )));
        runner.outputs.push_back(Err(AppError::new(
            ErrorCode::PlatformFailed,
            "rollback broke",
        )));
        let mut installer = make_installer(runner, &dir);

        let error = installer.install(&bundle).unwrap_err();

        assert!(error.message.contains("install broke"));
        assert!(error.message.contains("rollback failed: rollback broke"));
    }

    #[test]
    fn uninstall_boots_out_the_daemon_and_removes_both_files() {
        let dir = TestDir::new();
        let mut installer = make_installer(FakeRunner::default(), &dir);

        installer.uninstall().unwrap();

        let scripts = privileged_scripts(&installer.runner);
        assert_eq!(scripts.len(), 1);
        let layout = layout_in(&dir);
        assert!(scripts[0].contains(&format!("bootout system/{HELPER_LABEL}")));
        assert!(scripts[0].contains(&format!("rm -f '{}'", layout.daemon_plist().display())));
        assert!(scripts[0].contains(&format!("rm -f '{}'", layout.helper_binary().display())));
    }

    #[test]
    fn paths_with_shell_metacharacters_are_rejected_before_running() {
        let dir = TestDir::new();
        let mut bundle = bundle_in(&dir, b"verge-helper-binary");
        bundle.binary = dir.0.join("evil'; shutdown -h now; 'helper");
        fs::write(&bundle.binary, b"verge-helper-binary").unwrap();
        let mut installer = make_installer(FakeRunner::default(), &dir);

        let error = installer.install(&bundle).unwrap_err();

        assert_eq!(error.code, ErrorCode::InvalidInput);
        // 只有一次残留清理调用（失败发生在安装脚本构造阶段）；不含注入内容。
        let scripts = privileged_scripts(&installer.runner);
        assert_eq!(scripts.len(), 1);
        assert!(!scripts[0].contains("evil"));
    }

    #[test]
    fn discover_bundled_helper_reads_binary_and_build_time_digest() {
        let dir = TestDir::new();
        let resources = dir.0.join("Resources");
        fs::create_dir_all(resources.join("helper")).unwrap();
        fs::write(resources.join("helper/verge-helper"), b"helper").unwrap();
        fs::write(
            resources.join("helper/verge-helper.sha256"),
            format!("{:x}\n", Sha256::digest(b"helper")),
        )
        .unwrap();
        let bundle = discover_bundled_helper(&resources).unwrap();
        assert_eq!(bundle.binary, resources.join("helper/verge-helper"));
        assert_eq!(
            bundle.expected_sha256,
            format!("{:x}", Sha256::digest(b"helper"))
        );

        let missing = discover_bundled_helper(&dir.0.join("empty")).unwrap_err();
        assert_eq!(missing.code, ErrorCode::PlatformFailed);
    }

    #[test]
    fn relative_layout_is_rejected() {
        let dir = TestDir::new();
        let layout = HelperInstallLayout {
            install_dir: PathBuf::from("relative"),
            ..layout_in(&dir)
        };
        let result =
            MacHelperInstaller::new(FakeRunner::default(), layout, dir.0.join("staging"), 501);
        assert_eq!(
            result.err().map(|error| error.code),
            Some(ErrorCode::InvalidInput)
        );
    }
}
