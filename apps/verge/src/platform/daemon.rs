//! 双进程守护相关的平台辅助：socket 路径、守护进程拉起、无窗口 AppKit 事件循环。

use std::{
    fs,
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// 守护进程 Unix socket 的固定路径（数据目录内，用户私有，权限 0600）。
pub fn daemon_socket_path(data_dir: &Path) -> PathBuf {
    data_dir.join("daemon.sock")
}

/// 把当前进程的 stderr 重定向到 `{data_dir}/logs/verge.log`。
///
/// 守护进程与 GUI 进程都调用（双击启动时没有终端，日志必须落盘才能排查）。
/// 用 `dup2` 把 stderr 文件描述符指向日志文件；此后所有 `eprintln!` / panic
/// 输出都会写入该文件。
pub fn redirect_stderr_to_log(data_dir: &Path) -> io::Result<std::path::PathBuf> {
    let path = data_dir.join("logs").join("verge.log");
    redirect_stderr(&path)?;
    Ok(path)
}

/// 把 stderr 重定向到指定文件。
pub fn redirect_stderr(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        let fd = file.as_raw_fd();
        if unsafe { libc::dup2(fd, libc::STDERR_FILENO) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "stderr redirection is only supported on unix platforms",
        ))
    }
}

/// 拉起守护进程：以 `--daemon` 参数重新执行当前可执行文件，并使其
/// 完全脱离当前进程（新进程组、stdio 重定向），保证 GUI
/// 进程退出或被杀时守护进程不受信号波及。
pub fn spawn_daemon() -> io::Result<()> {
    let executable = std::env::current_exe()?;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Command::new(executable)
            .arg("--daemon")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = executable;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "daemon spawning is only supported on unix platforms",
        ))
    }
}

/// 在主线程运行无窗口 AppKit 事件循环，激活策略设为 `.accessory`
/// （只有菜单栏图标，无 Dock 图标）。阻塞直到进程退出。
///
/// 守护进程没有窗口，不加载 GPUI；这个循环只负责喂 NSStatusItem 与菜单事件。
#[cfg(target_os = "macos")]
pub fn run_accessory_appkit_loop() {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use objc2_foundation::MainThreadMarker;

    let marker = MainThreadMarker::new().expect("daemon main must run on the main thread");
    let app = NSApplication::sharedApplication(marker);
    // spawn 出来的守护进程不走 LaunchServices，Info.plist 的 LSUIElement 管不到它，
    // 激活策略必须在代码里设置。
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    app.run();
}

/// 非 macOS 平台暂无托盘型守护，占位（进程会在 main 分叉处报错退出）。
#[cfg(not(target_os = "macos"))]
pub fn run_accessory_appkit_loop() {
    eprintln!("Verge daemon mode is only supported on macOS");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_socket_path_is_inside_data_dir() {
        let path = daemon_socket_path(Path::new("/tmp/verge-data"));
        assert_eq!(path, PathBuf::from("/tmp/verge-data/daemon.sock"));
    }
}
