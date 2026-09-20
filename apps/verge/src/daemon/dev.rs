use super::*;

impl Backend {
    pub(super) fn check_dev_request(&self, request: &UiRequest) -> Result<(), AppError> {
        if !self.config.channel.is_dev() {
            return Ok(());
        }
        let blocked = match request {
            UiRequest::SystemProxy(command) => !matches!(command, SystemProxyCommand::GetState),
            UiRequest::Runtime(RuntimeCommand::SetNetworkSettings { settings }) => {
                settings.tun_enabled
            }
            UiRequest::Profile(command) => matches!(
                command,
                AppCommand::InstallHelper
                    | AppCommand::UninstallHelper
                    | AppCommand::UpdateApplication
                    | AppCommand::UpdateSystemProxySettings { .. }
            ),
            _ => false,
        };
        if blocked {
            Err(crate::identity::dev_restriction())
        } else {
            Ok(())
        }
    }
}

/// A maintenance connection does not claim/activate the primary GUI. Its single
/// operation is restricted to stopping a daemon that advertises the Dev identity.
pub fn stop_dev_daemon() -> Result<(), AppError> {
    use crate::ipc::{DaemonMessage, frame};
    use std::io::{BufReader, BufWriter};
    use std::os::unix::net::UnixStream;
    if !crate::identity::AppChannel::current().is_dev() {
        return Err(crate::identity::dev_restriction());
    }
    let directory = data_directory()?;
    let socket = daemon_socket_path(&directory);
    let stream = match UnixStream::connect(&socket) {
        Ok(stream) => stream,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            // No socket can also mean the daemon is still starting. Never replace
            // an active instance's bundle just because its listener is not ready.
            if directory.exists() && SingleInstance::acquire(&directory).is_err() {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "Verge Dev is starting or stopping; retry shortly",
                ));
            }
            return Ok(());
        }
        Err(e) => return Err(stop_error(e)),
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(12)))
        .map_err(stop_error)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(stop_error)?;
    let mut reader = BufReader::new(stream.try_clone().map_err(stop_error)?);
    let mut writer = BufWriter::new(stream);
    frame::write_message(
        &mut writer,
        &DaemonMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
            app_version: env!("CARGO_PKG_VERSION").into(),
            maintenance: true,
            channel: Some("dev".into()),
        },
    )
    .map_err(stop_error)?;
    match frame::read_message::<ClientMessage>(&mut reader).map_err(stop_error)? {
        ClientMessage::Welcome { initial, .. }
            if initial.capabilities.iter().any(|s| s == "dev_instance_v1") => {}
        _ => {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Refusing to stop a daemon without Dev maintenance support",
            ));
        }
    }
    frame::write_message(
        &mut writer,
        &DaemonMessage::Request(crate::gui::quit_request()),
    )
    .map_err(stop_error)?;
    loop {
        match frame::read_message::<ClientMessage>(&mut reader) {
            Ok(ClientMessage::Closed { reason }) if reason == "daemon_shutdown" => break,
            Ok(ClientMessage::Response(response)) => {
                if let UiResponse::Profile {
                    result: Err(error), ..
                } = response.response
                {
                    return Err(error);
                }
            }
            Ok(_) => {}
            Err(e) => return Err(stop_error(e)),
        }
    }
    drop(reader);
    drop(writer);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if !socket.exists() && SingleInstance::acquire(&directory).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Dev daemon did not finish shutting down",
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn stop_error(error: impl std::fmt::Display) -> AppError {
    AppError::new(
        ErrorCode::PlatformFailed,
        format!("Cannot stop Verge Dev: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejected_handshake_cannot_quit_a_stable_daemon_with_a_queued_request() {
        use crate::ipc::{DaemonMessage, frame};
        use std::os::unix::net::UnixStream;
        let (mut backend, dir) = super::super::tests::test_backend("dev-rejected");
        let socket = PathBuf::from(format!("/tmp/verge-rejected-{}.sock", std::process::id()));
        let (server, incoming) = IpcServer::bind(&socket).unwrap();
        server.spawn_accept();
        let mut stream = UnixStream::connect(&socket).unwrap();
        frame::write_message(
            &mut stream,
            &DaemonMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
                app_version: "test".into(),
                maintenance: true,
                channel: Some("dev".into()),
            },
        )
        .unwrap();
        frame::write_message(
            &mut stream,
            &DaemonMessage::Request(crate::gui::quit_request()),
        )
        .unwrap();
        let IpcServerEvent::Connected { conn_id, .. } =
            incoming.recv_timeout(Duration::from_secs(3)).unwrap()
        else {
            panic!("expected hello")
        };
        handle_daemon_connected(
            &server,
            &backend,
            conn_id,
            PROTOCOL_VERSION,
            true,
            Some("dev"),
        );
        assert!(!server.is_connected(conn_id));
        let (tx, _) = mpsc::channel();
        handle_daemon_request(
            &mut backend,
            &server,
            &mut CommandBus::default(),
            &tx,
            conn_id,
            crate::gui::quit_request(),
            &mut 0,
        );
        assert!(!backend.quit_requested);
        server.shutdown();
        drop(stream);
        drop(backend);
        let _ = fs::remove_dir_all(dir);
    }
    #[test]
    fn maintenance_keeps_primary_gui_and_rejects_other_channels() {
        use crate::ipc::{DaemonMessage, frame};
        use std::os::unix::net::UnixStream;
        let (mut backend, dir) = super::super::tests::test_backend("dev-handshake");
        backend.config.channel = crate::identity::AppChannel::Dev;
        let socket = PathBuf::from(format!(
            "/tmp/verge-dev-handshake-{}.sock",
            std::process::id()
        ));
        let (server, incoming) = IpcServer::bind(&socket).unwrap();
        server.spawn_accept();
        let handshake = |channel: &str, maintenance| {
            let mut stream = UnixStream::connect(&socket).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            frame::write_message(
                &mut stream,
                &DaemonMessage::Hello {
                    protocol_version: PROTOCOL_VERSION,
                    app_version: "test".into(),
                    maintenance,
                    channel: Some(channel.into()),
                },
            )
            .unwrap();
            let conn_id = loop {
                if let IpcServerEvent::Connected { conn_id, .. } =
                    incoming.recv_timeout(Duration::from_secs(3)).unwrap()
                {
                    break conn_id;
                }
            };
            handle_daemon_connected(
                &server,
                &backend,
                conn_id,
                PROTOCOL_VERSION,
                maintenance,
                Some(channel),
            );
            let message: ClientMessage = frame::read_message(&mut stream).unwrap();
            (stream, conn_id, message)
        };
        let (gui, primary, welcome) = handshake("dev", false);
        assert!(matches!(welcome, ClientMessage::Welcome { .. }));
        let (mut maintenance, _, welcome) = handshake("dev", true);
        assert!(matches!(welcome, ClientMessage::Welcome { .. }));
        assert_eq!(server.primary(), Some(primary));
        let (_, _, refused) = handshake("stable", false);
        assert!(matches!(refused, ClientMessage::Closed { .. }));
        assert_eq!(server.primary(), Some(primary));
        frame::write_message(
            &mut maintenance,
            &DaemonMessage::Request(crate::gui::quit_request()),
        )
        .unwrap();
        let request = loop {
            if let IpcServerEvent::Request { envelope, .. } =
                incoming.recv_timeout(Duration::from_secs(3)).unwrap()
            {
                break envelope.request;
            }
        };
        assert!(matches!(
            request,
            UiRequest::Profile(AppCommand::QuitApplication)
        ));
        server.shutdown();
        drop(gui);
        drop(maintenance);
        drop(backend);
        let _ = fs::remove_dir_all(dir);
    }
    #[test]
    fn dev_rejects_system_mutations_even_without_a_core() {
        let (mut backend, dir) = super::super::tests::test_backend("dev-policy");
        backend.config.channel = crate::identity::AppChannel::Dev;
        for command in [
            AppCommand::InstallHelper,
            AppCommand::UninstallHelper,
            AppCommand::UpdateApplication,
        ] {
            assert_eq!(
                backend.execute_profile(command).unwrap_err().code,
                ErrorCode::PermissionDenied
            );
        }
        for command in [
            SystemProxyCommand::Disable,
            SystemProxyCommand::SetEnabled { enabled: false },
            SystemProxyCommand::RecoverPending,
        ] {
            assert_eq!(
                backend.execute_system_proxy(command).unwrap_err().code,
                ErrorCode::PermissionDenied
            );
        }
        let settings = ApplicationSettings {
            launch_at_login: true,
            ..Default::default()
        };
        assert_eq!(
            backend.persist_settings(&settings).unwrap_err().code,
            ErrorCode::PermissionDenied
        );
        let settings = ApplicationSettings {
            language: "en".into(),
            ..Default::default()
        };
        backend.persist_settings(&settings).unwrap();
        assert_eq!(backend.settings.get().language, "en");
        drop(backend);
        let _ = fs::remove_dir_all(dir);
    }
}
