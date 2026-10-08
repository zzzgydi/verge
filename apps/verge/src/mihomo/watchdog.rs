//! An unprivileged parent for a leased core. EOF on fd 4 means the daemon died.
//! This process owns no routes and only ever terminates the child it spawned.
use std::{
    io,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{net::UnixStream, process::CommandExt},
    },
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

fn duplicate(fd: &impl AsRawFd) -> io::Result<OwnedFd> {
    // Reserve above stdio and our two inherited slots before remapping in the child.
    // SAFETY: F_DUPFD_CLOEXEC creates a new owned descriptor or returns -1.
    let raw = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 10) };
    if raw < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { OwnedFd::from_raw_fd(raw) })
    }
}

pub(super) fn command(binary: &std::path::Path, fd: &OwnedFd) -> io::Result<(Command, UnixStream)> {
    let (parent, child) = UnixStream::pair()?;
    let device = duplicate(fd)?;
    let watch = duplicate(&child)?;
    let mut command = Command::new(std::env::current_exe()?);
    command.arg("--core-watch").arg(binary);
    // Keep the watchdog and core in an owned group so the daemon can also
    // recover if the watchdog itself is killed or stops responding.
    command.process_group(0);
    // SAFETY: only async-signal-safe fd operations run after fork; owned captures
    // keep sources alive until spawn completes, including fd 3/4 collision cases.
    unsafe {
        command.pre_exec(move || {
            if libc::dup2(device.as_raw_fd(), 3) < 0 || libc::dup2(watch.as_raw_fd(), 4) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok((command, parent))
}

pub(super) fn exited(child: &Child) -> io::Result<bool> {
    // Observe without reaping. The leader's unreaped PID reserves the group ID
    // until we have killed remaining members; a recycled PID is never signalled.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            child.id(),
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result == 0 {
        Ok(info.si_signo != 0)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Only call for an unreaped child spawned with process_group(0).
pub(super) fn kill_group(child: &Child) -> io::Result<()> {
    let result = signal(-(child.id() as libc::pid_t), libc::SIGKILL);
    // Darwin returns EPERM (rather than ESRCH) when a group contains only
    // zombies. Our pinned, unprivileged children cannot gain another UID.
    #[cfg(target_os = "macos")]
    if result.as_ref().err().and_then(io::Error::raw_os_error) == Some(libc::EPERM)
        && exited(child)?
    {
        return Ok(());
    }
    result
}

fn signal(pid: libc::pid_t, signal: libc::c_int) -> io::Result<()> {
    if unsafe { libc::kill(pid, signal) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error)
    }
}

pub(super) fn stop_group(child: &mut Child, timeout: Duration) -> io::Result<()> {
    let _ = (|| {
        if !exited(child)?
            && let Err(error) = signal(child.id() as libc::pid_t, libc::SIGTERM)
            && !exited(child)?
        {
            return Err(error);
        }
        let deadline = Instant::now() + timeout;
        while !exited(child)? && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok::<_, io::Error>(())
    })();
    kill_group(child)?;
    child.wait()?;
    Ok(())
}

static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
extern "C" fn stop(_: libc::c_int) {
    STOP.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn run() -> io::Result<()> {
    use std::io::Read;
    let mut arguments = std::env::args_os().skip(2);
    let binary = arguments
        .next()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: this private entry is spawned with owned descriptors 3 and 4.
    // Check first so a direct CLI invocation fails without touching unrelated fds.
    if unsafe { libc::fcntl(3, libc::F_GETFD) } < 0 || unsafe { libc::fcntl(4, libc::F_GETFD) } < 0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Missing core lease descriptors",
        ));
    }
    let device = unsafe { OwnedFd::from_raw_fd(3) };
    let mut watch = unsafe { UnixStream::from_raw_fd(4) };
    // The core inherits only the device. It must not keep the daemon lifeline alive.
    if unsafe { libc::fcntl(4, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error());
    }
    watch.set_read_timeout(Some(Duration::from_millis(100)))?;
    // SAFETY: the signal handler performs a lock-free atomic store only.
    unsafe {
        libc::signal(libc::SIGTERM, stop as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, stop as *const () as libc::sighandler_t);
    }
    let mut child = Command::new(binary)
        .args(arguments)
        .stdin(Stdio::null())
        .spawn()?;
    drop(device);
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(io::Error::other("Leased core exited"))
                };
            }
            Err(error) => break Err(error),
            Ok(None) => {}
        }
        if STOP.load(std::sync::atomic::Ordering::Relaxed) {
            break Ok(());
        }
        match watch.read(&mut [0]) {
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            _ => break Ok(()),
        }
    };
    // A broken lease must release the device promptly; no graceful network traffic
    // is possible after the owning daemon exits. Kill only our unreaped child.
    let _ = child.kill();
    let _ = child.wait();
    result
}
