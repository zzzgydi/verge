//! No utun or privileged actions: socket pairs stand in for the device and lease.
use std::{
    io::{BufRead, BufReader, Read},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{net::UnixStream, process::CommandExt},
    },
    process::{Command, Stdio},
    time::{Duration, Instant},
};
fn duplicate(fd: &impl AsRawFd) -> OwnedFd {
    let raw = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 20) };
    assert!(raw >= 0);
    unsafe { OwnedFd::from_raw_fd(raw) }
}
#[test]
fn tun_watchdog_inherits_device_and_reaps_core_when_daemon_disconnects_or_stops() {
    for term in [false, true] {
        let (source, mut peer) = UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let (life, watch) = UnixStream::pair().unwrap();
        let device = duplicate(&source);
        let signal = duplicate(&watch);
        drop(source);
        drop(watch);
        let mut command = Command::new(env!("CARGO_BIN_EXE_verge-gpui"));
        command
            .args([
                "--core-watch",
                "/bin/sh",
                "-c",
                "printf '%s\\n' $$ >&3; exec /bin/sleep 60",
            ])
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        unsafe {
            command.pre_exec(move || {
                if libc::dup2(device.as_raw_fd(), 3) < 0 || libc::dup2(signal.as_raw_fd(), 4) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut watchdog = command.spawn().unwrap();
        drop(command);
        let mut pid = String::new();
        BufReader::with_capacity(1, &mut peer)
            .read_line(&mut pid)
            .unwrap();
        let core: u32 = pid.trim().parse().unwrap();
        assert_eq!(unsafe { libc::getpgid(core as i32) }, watchdog.id() as i32);
        if term {
            unsafe { libc::kill(watchdog.id() as i32, libc::SIGTERM) };
        } else {
            drop(life);
        }
        assert_eq!(peer.read(&mut [0]).unwrap(), 0, "core retained the device");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = watchdog.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if Instant::now() > deadline {
                let _ = watchdog.kill();
                let _ = watchdog.wait();
                panic!("watchdog did not exit");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            unsafe { libc::kill(core as i32, 0) },
            -1,
            "unreaped core survived"
        );
    }
}
