use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::Path,
};

pub use verge_helper_protocol::{
    HelperRequest, HelperResponse, MAX_REQUEST_BYTES, PROTOCOL_VERSION,
};

pub fn serve_connection(mut stream: UnixStream, allowed_uid: u32) -> std::io::Result<()> {
    let peer_uid = peer_uid(&stream)?;
    if peer_uid != allowed_uid && peer_uid != 0 {
        return write_response(
            &mut stream,
            &HelperResponse::Error {
                code: "unauthorized_peer".into(),
                message: "peer uid is not authorized".into(),
            },
        );
    }
    let mut line = String::new();
    BufReader::new(stream.try_clone()?)
        .take(MAX_REQUEST_BYTES)
        .read_line(&mut line)?;
    let request = match serde_json::from_str::<HelperRequest>(&line) {
        Ok(request) => request,
        Err(error) => {
            return write_response(
                &mut stream,
                &HelperResponse::Error {
                    code: "invalid_request".into(),
                    message: error.to_string(),
                },
            );
        }
    };
    let response = match request {
        HelperRequest::Ping { protocol_version } if protocol_version == PROTOCOL_VERSION => {
            HelperResponse::Pong { protocol_version }
        }
        HelperRequest::Ping { .. } => HelperResponse::Error {
            code: "protocol_mismatch".into(),
            message: format!("helper protocol version is {PROTOCOL_VERSION}"),
        },
        HelperRequest::GetCapabilities => HelperResponse::Capabilities {
            protocol_version: PROTOCOL_VERSION,
            tun_lifecycle: false,
        },
    };
    write_response(&mut stream, &response)
}

pub fn prepare_socket_path(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

pub fn restrict_socket(path: &Path) -> std::io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

fn write_response(stream: &mut UnixStream, response: &HelperResponse) -> std::io::Result<()> {
    serde_json::to_writer(&mut *stream, response)?;
    stream.write_all(b"\n")?;
    stream.flush()
}

#[cfg(target_os = "macos")]
fn peer_uid(stream: &UnixStream) -> std::io::Result<u32> {
    use std::os::fd::AsRawFd;

    let mut uid = 0;
    let mut gid = 0;
    // SAFETY: getpeereid writes to valid uid/gid pointers for this connected Unix socket fd.
    let result = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    if result == 0 {
        Ok(uid)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn peer_uid(_stream: &UnixStream) -> std::io::Result<u32> {
    // SAFETY: getuid has no preconditions.
    Ok(unsafe { libc::getuid() })
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Write},
        os::unix::net::UnixStream,
    };

    use super::*;

    fn exchange(request: &HelperRequest, allowed_uid: u32) -> HelperResponse {
        let (mut client, server) = UnixStream::pair().unwrap();
        let handle = std::thread::spawn(move || serve_connection(server, allowed_uid).unwrap());
        serde_json::to_writer(&mut client, request).unwrap();
        client.write_all(b"\n").unwrap();
        let mut response = String::new();
        BufReader::new(client).read_line(&mut response).unwrap();
        handle.join().unwrap();
        serde_json::from_str(&response).unwrap()
    }

    #[test]
    fn ping_negotiates_an_exact_protocol_version() {
        // SAFETY: getuid has no preconditions.
        let uid = unsafe { libc::getuid() };
        assert_eq!(
            exchange(
                &HelperRequest::Ping {
                    protocol_version: PROTOCOL_VERSION,
                },
                uid,
            ),
            HelperResponse::Pong {
                protocol_version: PROTOCOL_VERSION,
            }
        );
        assert!(matches!(
            exchange(
                &HelperRequest::Ping {
                    protocol_version: PROTOCOL_VERSION + 1,
                },
                uid,
            ),
            HelperResponse::Error { code, .. } if code == "protocol_mismatch"
        ));
    }

    #[test]
    fn capabilities_do_not_claim_unimplemented_privileged_writes() {
        // SAFETY: getuid has no preconditions.
        let uid = unsafe { libc::getuid() };
        assert_eq!(
            exchange(&HelperRequest::GetCapabilities, uid),
            HelperResponse::Capabilities {
                protocol_version: PROTOCOL_VERSION,
                tun_lifecycle: false,
            }
        );
    }
}
