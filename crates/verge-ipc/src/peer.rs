//! Unix socket 对端身份校验。
//!
//! 守护进程只接受同一用户的连接。macOS 上用 `LOCAL_PEERCRED` 取对端 uid；
//! 其它平台暂不支持，返回错误（守护进程会拒绝连接）。

use std::io;
#[cfg(unix)]
use std::os::unix::net::UnixStream;

/// 取对端进程 uid。macOS 上通过 `getsockopt(LOCAL_PEERCRED)` 获得。
#[cfg(all(unix, target_os = "macos"))]
pub fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    use std::os::fd::AsRawFd;

    let mut credential: libc::xucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::xucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERCRED,
            &mut credential as *mut _ as *mut libc::c_void,
            &mut length,
        )
    };
    if result == 0 {
        Ok(credential.cr_uid)
    } else {
        Err(io::Error::last_os_error())
    }
}

/// 非 macOS 平台暂不支持对端校验。
#[cfg(not(target_os = "macos"))]
pub fn peer_uid(_stream: &UnixStream) -> io::Result<u32> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "peer uid verification is only supported on macOS",
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    fn peer_uid_is_never_zero_for_self_connection_on_macos() {
        #[cfg(target_os = "macos")]
        {
            use std::os::unix::net::UnixStream;

            let (a, b) = UnixStream::pair().unwrap();
            let uid = super::peer_uid(&a).unwrap();
            let expected = unsafe { libc::geteuid() };
            assert_eq!(uid, expected);
            drop(b);
        }
    }
}
