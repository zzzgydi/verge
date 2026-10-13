use super::*;

impl Backend {
    pub(super) fn optimization_context(&self) -> crate::optimization::Context {
        let (errors, connections) = self.optimization.observations();
        crate::optimization::Context {
            socket: self.config.internal_socket(),
            profiles: self.profiles.clone(),
            services: self.config.services.clone(),
            recovery: self.config.recovery_path.clone(),
            target: self.proxy_session.target.clone(),
            tun_connected: self
                .tun_lease
                .as_ref()
                .is_some_and(|lease| lease.connected()),
            errors,
            connections,
            epoch: self.optimization.epoch.clone(),
            active_probes: self.optimization.snapshot().active_probes,
            sample_cursor: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optimization_commands_from_agents_are_rejected_before_execution() {
        use crate::ipc::{DaemonMessage, frame};
        let (mut backend, dir) = super::super::tests::test_backend("optimization-actor");
        let socket = PathBuf::from(format!("/tmp/verge-health-{}.sock", std::process::id()));
        let (server, incoming) = IpcServer::bind(&socket).unwrap();
        server.spawn_accept();
        let mut stream = std::os::unix::net::UnixStream::connect(&socket).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        frame::write_message(
            &mut stream,
            &DaemonMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
                app_version: "test".into(),
                maintenance: false,
                channel: "stable".into(),
            },
        )
        .unwrap();
        let IpcServerEvent::Connected { conn_id, .. } =
            incoming.recv_timeout(Duration::from_secs(3)).unwrap()
        else {
            panic!("hello expected")
        };
        let (tx, _) = mpsc::channel();
        let request = UiRequestEnvelope {
            request_id: 1,
            operation_id: 1,
            operation_index: 0,
            operation_len: 1,
            context: crate::domain::CommandContext {
                actor: crate::domain::CommandActor::Agent,
                approval: Some(crate::domain::CommandApproval {
                    max_risk: crate::domain::CommandRisk::Destructive,
                }),
            },
            request: UiRequest::Optimization(crate::optimization::Command::Configure {
                observe: true,
                model_analysis: true,
            }),
        };
        handle_daemon_request(
            &mut backend,
            &server,
            &mut CommandBus::default(),
            &tx,
            conn_id,
            request,
            &mut 0,
        );
        assert!(!backend.optimization.snapshot().observe);
        assert!(backend.optimization_connections.is_empty());
        let response: ClientMessage = frame::read_message(&mut stream).unwrap();
        assert!(matches!(
            response,
            ClientMessage::Response(UiResponseEnvelope {
                response: UiResponse::Optimization { result: Err(_), .. },
                ..
            })
        ));
        server.shutdown();
        drop(stream);
        drop(backend);
        let _ = fs::remove_dir_all(dir);
    }
}
