use super::*;
use std::{os::fd::AsFd, time::Duration};

pub(super) fn serve(
    mut stream: UnixStream,
    tun: &SharedTunBackend,
    ipv6: bool,
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(60)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let prepared = (|| {
        let mut backend = lock_tun(tun);
        if !backend.tun_fd_lease_supported() {
            return Err(HelperFailure::new(
                "tun_unsupported",
                "Descriptor leases unavailable",
            ));
        }
        let mut config = TunConfig::verge_default();
        config.routes.clear();
        // Mihomo derives its TUN address from fake-ip-range with a /30 prefix.
        config.netmask = "255.255.255.252".into();
        let device =
            backend.enable_tun(&validate_tun_config(&config).map_err(validated_failure)?)?;
        match backend.descriptor(&device) {
            Ok(fd) => Ok((device, fd)),
            Err(error) => {
                let _ = backend.disable_tun(Some(&device));
                Err(error)
            }
        }
    })();
    let (device, fd) = match prepared {
        Ok(ready) => ready,
        Err(error) => {
            verge_helper_protocol::descriptor::send(&stream, None)?;
            return write_response(&mut stream, &failure_response(error));
        }
    };
    let mut dns = None;
    let result = (|| {
        verge_helper_protocol::descriptor::send(&stream, Some(fd.as_fd()))?;
        drop(fd);
        write_response(
            &mut stream,
            &HelperResponse::TunEnabled {
                device: device.clone(),
            },
        )?;
        let mut signal = [0];
        stream.read_exact(&mut signal)?;
        if signal == *b"Q" {
            return Ok(());
        }
        if signal != *b"R" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid readiness signal",
            ));
        }
        let result = (|| {
            let mut backend = lock_tun(tun);
            backend.activate(&device, ipv6)?;
            dns = Some(backend.dns_session(&device)?);
            Ok::<_, HelperFailure>(())
        })();
        match result {
            Ok(()) => write_response(
                &mut stream,
                &HelperResponse::TunEnabled {
                    device: device.clone(),
                },
            )?,
            Err(error) => {
                write_response(&mut stream, &failure_response(error))?;
                return Ok(());
            }
        }
        stream.set_read_timeout(None)?;
        // EOF or any subsequent byte releases this lease. No detached device survives a client.
        let _ = stream.read(&mut signal);
        Ok(())
    })();
    drop(dns);
    let cleanup = lock_tun(tun).disable_tun(Some(&device));
    match cleanup {
        Ok(()) => {
            let _ = write_response(&mut stream, &HelperResponse::TunDisabled);
        }
        Err(error) => {
            let _ = write_response(&mut stream, &failure_response(error));
        }
    }
    result
}

#[cfg(target_os = "macos")]
pub(super) fn activate(
    device: &str,
    ipv6: bool,
    owned: &mut Vec<(String, bool)>,
) -> Result<(), HelperFailure> {
    if ipv6 {
        run_fixed(
            "/sbin/ifconfig",
            &[
                device.into(),
                "inet6".into(),
                "fdfe:dcba:9876::1/126".into(),
                "alias".into(),
            ],
        )?;
    }
    for (route, v6) in [
        ("0.0.0.0/1", false),
        ("128.0.0.0/1", false),
        ("::/1", true),
        ("8000::/1", true),
    ] {
        if v6 && !ipv6 {
            continue;
        }
        run_fixed(
            "/sbin/route",
            &[
                "-n".into(),
                "add".into(),
                if v6 { "-inet6" } else { "-inet" }.into(),
                route.into(),
                "-interface".into(),
                device.into(),
            ],
        )?;
        owned.push((route.into(), v6));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(super) fn deactivate(device: &str) -> Result<(), HelperFailure> {
    // Stop capture even when a client still has a descriptor. Device teardown
    // removes interface-owned routes after the daemon/core close their copies.
    run_fixed("/sbin/ifconfig", &[device.into(), "down".into()])
}
#[cfg(not(target_os = "macos"))]
pub(super) fn activate(_: &str, _: bool, _: &mut Vec<(String, bool)>) -> Result<(), HelperFailure> {
    Err(HelperFailure::new("tun_unsupported", "macOS only"))
}
#[cfg(not(target_os = "macos"))]
pub(super) fn deactivate(_: &str) -> Result<(), HelperFailure> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::{AsFd, OwnedFd};
    struct Fake {
        calls: Arc<Mutex<Vec<&'static str>>>,
        fd: Option<OwnedFd>,
        peer: UnixStream,
        fail: bool,
    }
    impl TunBackend for Fake {
        fn tun_lifecycle_supported(&self) -> bool {
            true
        }
        fn tun_fd_lease_supported(&self) -> bool {
            true
        }
        fn enable_tun(&mut self, config: &ValidatedTun) -> Result<String, HelperFailure> {
            assert!(config.routes.is_empty());
            assert_eq!(config.netmask.to_string(), "255.255.255.252");
            self.calls.lock().unwrap().push("prepare");
            Ok("utun7".into())
        }
        fn descriptor(&self, _: &str) -> Result<OwnedFd, HelperFailure> {
            Ok(self
                .fd
                .as_ref()
                .unwrap()
                .as_fd()
                .try_clone_to_owned()
                .unwrap())
        }
        fn activate(&mut self, _: &str, ipv6: bool) -> Result<(), HelperFailure> {
            assert!(ipv6);
            self.calls.lock().unwrap().push("activate");
            if self.fail {
                Err(HelperFailure::new("injected", "route failure"))
            } else {
                Ok(())
            }
        }
        fn disable_tun(&mut self, _: Option<&str>) -> Result<(), HelperFailure> {
            self.fd.take();
            // The client must close its descriptor before requesting explicit cleanup.
            let mut byte = [0];
            assert_eq!(self.peer.read(&mut byte).unwrap(), 0);
            self.calls.lock().unwrap().push("release");
            Ok(())
        }
    }
    fn response(stream: &mut UnixStream) -> HelperResponse {
        let mut line = String::new();
        BufReader::with_capacity(1, stream)
            .read_line(&mut line)
            .unwrap();
        serde_json::from_str(&line).unwrap()
    }
    #[test]
    fn lease_waits_for_readiness_and_cleans_on_release_eof_and_activation_failure() {
        for scenario in ["release", "eof", "abort", "failure"] {
            let calls = Arc::new(Mutex::new(vec![]));
            let (source, peer) = UnixStream::pair().unwrap();
            peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
            let tun = shared_tun_backend(Fake {
                calls: calls.clone(),
                fd: Some(source.into()),
                peer,
                fail: scenario == "failure",
            });
            let (mut client, server) = UnixStream::pair().unwrap();
            let worker = std::thread::spawn(move || serve(server, &tun, true));
            let fd = verge_helper_protocol::descriptor::receive(&client)
                .unwrap()
                .unwrap();
            assert!(matches!(
                response(&mut client),
                HelperResponse::TunEnabled { .. }
            ));
            assert_eq!(*calls.lock().unwrap(), ["prepare"]);
            if scenario == "abort" {
                drop(fd);
                client.write_all(b"Q").unwrap();
            } else {
                // Drop before a failure response allows helper cleanup to prove ownership.
                drop(fd);
                client.write_all(b"R").unwrap();
                let ack = response(&mut client);
                assert_eq!(
                    matches!(ack, HelperResponse::Error { .. }),
                    scenario == "failure"
                );
                if scenario == "release" {
                    client.write_all(b"Q").unwrap();
                }
            }
            if scenario == "eof" {
                drop(client);
            } else {
                assert_eq!(response(&mut client), HelperResponse::TunDisabled);
            }
            worker.join().unwrap().unwrap();
            let expected = if scenario == "abort" {
                vec!["prepare", "release"]
            } else {
                vec!["prepare", "activate", "release"]
            };
            assert_eq!(*calls.lock().unwrap(), expected);
        }
    }
}
