use super::*;
use std::{os::fd::OwnedFd, time::Duration};

/// The socket owns the helper lease; drop closes it and releases helper routes.
pub(crate) struct TunLease {
    stream: UnixStream,
    pub device: String,
    pub fd: OwnedFd,
    cleaned: bool,
}
impl TunLease {
    pub fn prepare(socket: &Path, ipv6: bool) -> Result<Self, AppError> {
        let capabilities = MacHelperClient::new(socket).capabilities()?;
        if capabilities.protocol_version != PROTOCOL_VERSION || !capabilities.tun_fd_lease {
            return Err(platform_error(
                "Install or repair the helper to use TUN descriptor leases",
            ));
        }
        let mut stream = UnixStream::connect(socket).map_err(platform_io_error)?;
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .map_err(platform_io_error)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(platform_io_error)?;
        serde_json::to_writer(&mut stream, &HelperRequest::LeaseTun { ipv6 })
            .map_err(|_| platform_error("Cannot request TUN lease"))?;
        stream.write_all(b"\n").map_err(platform_io_error)?;
        let fd = verge_helper_protocol::descriptor::receive(&stream).map_err(platform_io_error)?;
        let response = read_response(&mut stream)?;
        match (response, fd) {
            (HelperResponse::TunEnabled { device }, Some(fd)) if valid_name(&device) => Ok(Self {
                stream,
                device,
                fd,
                cleaned: false,
            }),
            (HelperResponse::Error { code, message }, _) => Err(helper_call_error(&code, &message)),
            _ => Err(platform_error("Invalid TUN lease response")),
        }
    }
    pub fn activate(&mut self) -> Result<(), AppError> {
        self.stream.write_all(b"R").map_err(platform_io_error)?;
        match read_response(&mut self.stream)? {
            HelperResponse::TunEnabled { device } if device == self.device => Ok(()),
            HelperResponse::Error { code, message } => {
                self.cleaned = matches!(
                    read_response(&mut self.stream),
                    Ok(HelperResponse::TunDisabled)
                );
                Err(helper_call_error(&code, &message))
            }
            _ => Err(platform_error("Invalid TUN readiness response")),
        }
    }
    pub fn release(mut self) -> Result<(), AppError> {
        drop(self.fd);
        if self.cleaned {
            return Ok(());
        }
        self.stream.write_all(b"Q").map_err(platform_io_error)?;
        match read_response(&mut self.stream)? {
            HelperResponse::TunDisabled => Ok(()),
            HelperResponse::Error { code, message } => Err(helper_call_error(&code, &message)),
            _ => Err(platform_error("Invalid TUN cleanup response")),
        }
    }
    pub fn connected(&self) -> bool {
        use std::os::fd::AsRawFd;
        let mut byte = 0u8;
        // SAFETY: nonblocking peek reads at most one byte from this borrowed socket.
        let n = unsafe {
            libc::recv(
                self.stream.as_raw_fd(),
                (&raw mut byte).cast(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        };
        n < 0
            && matches!(
                io::Error::last_os_error().kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
            )
    }
}
fn valid_name(name: &str) -> bool {
    name.strip_prefix("utun").is_some_and(|n| {
        !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && n.parse::<u32>().is_ok()
    })
}
fn read_response(stream: &mut UnixStream) -> Result<HelperResponse, AppError> {
    let mut response = String::new();
    BufReader::with_capacity(1, stream)
        .take(MAX_REQUEST_BYTES)
        .read_line(&mut response)
        .map_err(platform_io_error)?;
    serde_json::from_str(&response).map_err(|_| platform_error("Invalid helper response"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::{fd::AsFd, unix::net::UnixListener},
        sync::{Arc, Mutex},
    };
    pub(crate) struct FakeTun {
        pub calls: Arc<Mutex<Vec<String>>>,
        pub fd: OwnedFd,
    }
    impl verge_helper::TunBackend for FakeTun {
        fn tun_lifecycle_supported(&self) -> bool {
            true
        }
        fn tun_fd_lease_supported(&self) -> bool {
            true
        }
        fn descriptor(&self, _: &str) -> Result<OwnedFd, verge_helper::HelperFailure> {
            Ok(self.fd.as_fd().try_clone_to_owned().unwrap())
        }
        fn enable_tun(
            &mut self,
            _: &verge_helper::ValidatedTun,
        ) -> Result<String, verge_helper::HelperFailure> {
            self.calls.lock().unwrap().push("prepare".into());
            Ok("utun9".into())
        }
        fn activate(&mut self, _: &str, ipv6: bool) -> Result<(), verge_helper::HelperFailure> {
            self.calls.lock().unwrap().push(format!("activate:{ipv6}"));
            Ok(())
        }
        fn disable_tun(&mut self, _: Option<&str>) -> Result<(), verge_helper::HelperFailure> {
            self.calls.lock().unwrap().push("release".into());
            Ok(())
        }
    }
    #[test]
    fn client_negotiates_descriptor_lease_and_releases_on_drop_or_explicitly() {
        for explicit in [false, true] {
            let path = std::env::temp_dir()
                .join(format!("verge-tun-{}-{explicit}.sock", std::process::id()));
            let _ = std::fs::remove_file(&path);
            let listener = UnixListener::bind(&path).unwrap();
            let calls = Arc::new(Mutex::new(vec![]));
            let (source, _peer) = UnixStream::pair().unwrap();
            let backend = verge_helper::shared_tun_backend(FakeTun {
                calls: calls.clone(),
                fd: source.into(),
            });
            let worker = std::thread::spawn(move || {
                for _ in 0..2 {
                    let (stream, _) = listener.accept().unwrap();
                    verge_helper::serve_connection(stream, verge_helper::current_uid(), &backend)
                        .unwrap();
                }
            });
            let mut lease = TunLease::prepare(&path, true).unwrap();
            assert!(lease.connected());
            assert_eq!(*calls.lock().unwrap(), ["prepare"]);
            lease.activate().unwrap();
            assert!(lease.connected());
            if explicit {
                lease.release().unwrap();
            } else {
                drop(lease);
            }
            worker.join().unwrap();
            assert_eq!(
                *calls.lock().unwrap(),
                ["prepare", "activate:true", "release"]
            );
            std::fs::remove_file(path).unwrap();
        }
    }
    #[test]
    fn old_helper_is_rejected_before_requesting_a_device() {
        let path = std::env::temp_dir().join(format!("verge-tun-old-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(&stream).read_line(&mut request).unwrap();
            stream
                .write_all(
                    b"{\"type\":\"capabilities\",\"protocol_version\":1,\"tun_lifecycle\":true}\n",
                )
                .unwrap();
        });
        assert!(
            TunLease::prepare(&path, false)
                .err()
                .unwrap()
                .message
                .contains("repair")
        );
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }
}
