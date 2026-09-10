//! Exercise the daemon dispatcher, including its background query workers.
use super::*;
use crate::domain::{
    CommandActor, CommandContext, ProfileId, ProfileSource, RunMode, UpdatePolicy,
};

struct TestDirectory(PathBuf);

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixture() -> (TestDirectory, BackendConfig) {
    let dir = TestDirectory(PathBuf::from(format!(
        "/tmp/verge-worker-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    let config = BackendConfig {
        data_dir: dir.0.clone(),
        binary: dir.0.join("unused-mihomo"),
        manifest: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/mihomo/manifest.json"),
        controller: available_controller().unwrap(),
        secret: "unused-legacy-test-secret".into(),
        // Supplying services skips discovery; this test never changes system proxy or TUN.
        services: vec!["Wi-Fi".into()],
        recovery_path: dir.0.join("recovery.json"),
        helper_socket: dir.0.join("unused-helper.sock"),
    };
    (dir, config)
}

#[test]
fn isolated_queries_and_delay_use_unix_transport() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    let (_dir, config) = fixture();
    fs::create_dir_all(config.internal_socket().parent().unwrap()).unwrap();
    let listener = UnixListener::bind(config.internal_socket()).unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        for path in [
            "/configs",
            "/proxies",
            "/rules",
            "/providers/proxies",
            "/providers/rules",
            "/configs",
            "/proxies/node/delay?",
        ] {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "no Unix request for {path}");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(&stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert!(line.starts_with(&format!("GET {path}")), "{line}");
            loop {
                line.clear();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
            }
            let body = r#"{"mode":"rule","proxies":{},"rules":[],"providers":{},"delay":23}"#;
            // A valid delay result can arrive after the ordinary 2 s controller deadline.
            if path.contains("/delay?") {
                std::thread::sleep(Duration::from_millis(2_500));
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        }
    });
    for command in [
        RuntimeCommand::GetMode,
        RuntimeCommand::ListProxyGroups,
        RuntimeCommand::ListRules,
        RuntimeCommand::ListProviders,
        RuntimeCommand::GetNetworkSettings,
        RuntimeCommand::TestProxyDelay {
            proxy: "node".into(),
            url: "https://example.invalid".into(),
            timeout_ms: 5_000,
        },
    ] {
        let response = execute_isolated_runtime(
            UiRequest::Runtime(command.clone()),
            config.build_runtime().unwrap(),
        );
        assert!(response_succeeded(&response), "{command:?}: {response:?}");
        if matches!(command, RuntimeCommand::TestProxyDelay { .. }) {
            let UiResponse::Runtime {
                result: Ok(result), ..
            } = response
            else {
                unreachable!()
            };
            assert_eq!(result.output, RuntimeCommandOutput::Delay(23));
        }
    }
    server.join().unwrap();
}

#[test]
#[ignore = "requires MIHOMO_BIN pointing to the pinned sidecar"]
fn daemon_queries_use_internal_socket_with_external_controller_disabled() {
    let (dir, mut config) = fixture();
    config.binary = env::var_os("MIHOMO_BIN").expect("MIHOMO_BIN").into();
    let mut backend = Backend::new(config).unwrap();
    let id = ProfileId::parse("worker-contract").unwrap();
    backend.profiles.import(
        Profile::new(id.clone(), "Worker contract", ProfileSource::Local,
            UpdatePolicy::Manual, 0, None).unwrap(),
        "mixed-port: 0\nmode: rule\nlog-level: warning\nrules: ['MATCH,DIRECT']\ndns:\n  enable: false\n",
    ).unwrap();
    backend.profiles.select(&id).unwrap();
    let mut engine = backend.build_engine(&id).unwrap();
    engine.supervisor.start().unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while engine
        .supervisor
        .health(Duration::from_millis(100))
        .is_err()
    {
        assert!(
            Instant::now() < deadline,
            "controller unavailable: {:?}",
            engine.supervisor.logs()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    // The primary runtime is healthy, while the old TCP controller is closed.
    assert!(
        RuntimeCommandHandler::new(&mut engine.runtime)
            .execute(RuntimeCommand::GetMode)
            .is_ok()
    );
    assert!(std::net::TcpStream::connect(backend.config.controller).is_err());
    backend.engine = Some(engine);
    let (server, _) = IpcServer::bind(&dir.0.join("daemon.sock")).unwrap();
    // Tray commands use the same controller and validated profile lifecycle with no GUI.
    for mode in [RunMode::Global, RunMode::Direct, RunMode::Rule] {
        super::tray::execute_inner(&mut backend, &server, TrayCommand::SetMode(mode)).unwrap();
        let response = backend.execute(UiRequest::Runtime(RuntimeCommand::GetMode));
        assert!(
            matches!(response, UiResponse::Runtime { result: Ok(crate::domain::RuntimeCommandResult { output: RuntimeCommandOutput::Mode(actual), .. }), .. } if actual == mode)
        );
    }
    super::tray::execute_inner(
        &mut backend,
        &server,
        TrayCommand::SelectProxy {
            group: "GLOBAL".into(),
            proxy: "DIRECT".into(),
        },
    )
    .unwrap();
    super::tray::execute_inner(&mut backend, &server, TrayCommand::RestartCore).unwrap();
    assert!(
        backend
            .engine
            .as_ref()
            .unwrap()
            .supervisor
            .health(Duration::from_secs(1))
            .is_ok()
    );
    let (tx, rx) = mpsc::channel();
    let mut bus = CommandBus::default();
    let mut jobs = 0;
    for (index, command) in [
        RuntimeCommand::GetMode,
        RuntimeCommand::ListProxyGroups,
        RuntimeCommand::ListRules,
        RuntimeCommand::ListProviders,
        RuntimeCommand::GetNetworkSettings,
    ]
    .into_iter()
    .enumerate()
    {
        let envelope = UiRequestEnvelope {
            request_id: index as u64,
            operation_id: index as u64,
            operation_index: 0,
            operation_len: 1,
            context: CommandContext {
                actor: CommandActor::UserInterface,
                approval: None,
            },
            request: UiRequest::Runtime(command.clone()),
        };
        handle_daemon_request(&mut backend, &server, &mut bus, &tx, 1, envelope, &mut jobs);
        assert_eq!(jobs, 1, "query must go through the worker");
        let DaemonEvent::WorkerDone { response, .. } =
            rx.recv_timeout(Duration::from_secs(5)).unwrap()
        else {
            panic!("expected a worker response");
        };
        let UiResponse::Runtime { request, result } = *response else {
            panic!("expected a runtime response");
        };
        assert_eq!(request, command);
        let result = result.unwrap_or_else(|error| panic!("{command:?}: {error}"));
        match command {
            RuntimeCommand::GetMode => {
                assert_eq!(result.output, RuntimeCommandOutput::Mode(RunMode::Rule))
            }
            RuntimeCommand::ListProxyGroups => {
                let RuntimeCommandOutput::ProxyGroups(snapshot) = result.output else {
                    panic!("expected proxy snapshot");
                };
                assert!(snapshot.groups.iter().any(|group| group.name == "GLOBAL"));
                assert_eq!(snapshot.proxies["DIRECT"].kind, "Direct");
                assert_eq!(snapshot.proxies["DIRECT"].udp, Some(true));
            }
            RuntimeCommand::ListRules => assert!(
                matches!(result.output, RuntimeCommandOutput::Rules(rules) if rules.len() == 1)
            ),
            RuntimeCommand::ListProviders => {
                assert!(matches!(result.output, RuntimeCommandOutput::Providers(_)))
            }
            RuntimeCommand::GetNetworkSettings => assert!(matches!(
                result.output,
                RuntimeCommandOutput::NetworkSettings(_)
            )),
            _ => unreachable!(),
        }
        jobs -= 1;
    }
    // Test the real node-delay endpoint through the same dispatcher, without external traffic.
    // The first succeeds after the old 2 s socket deadline; the second exceeds Mihomo's deadline.
    for (index, timeout_ms, target_delay_ms) in [(10, 5_000, 2_500), (11, 100, 350)] {
        use std::io::{BufRead, BufReader, Write};
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = target.local_addr().unwrap();
        target.set_nonblocking(true).unwrap();
        let target_server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match target.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "Mihomo did not reach delay target"
                        );
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(&stream);
            loop {
                let mut line = String::new();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(target_delay_ms));
            let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n");
        });
        let command = RuntimeCommand::TestProxyDelay {
            proxy: "DIRECT".into(),
            url: format!("http://{address}/generate_204"),
            timeout_ms,
        };
        let envelope = UiRequestEnvelope {
            request_id: index,
            operation_id: index,
            operation_index: 0,
            operation_len: 1,
            context: CommandContext {
                actor: CommandActor::UserInterface,
                approval: None,
            },
            request: UiRequest::Runtime(command),
        };
        handle_daemon_request(&mut backend, &server, &mut bus, &tx, 1, envelope, &mut jobs);
        assert_eq!(jobs, 1);
        let result = rx.recv_timeout(Duration::from_secs(8));
        target_server.join().unwrap();
        let DaemonEvent::WorkerDone { response, .. } = result.unwrap() else {
            panic!("expected worker response")
        };
        let UiResponse::Runtime { result, .. } = *response else {
            panic!("expected runtime response")
        };
        if timeout_ms == 5_000 {
            assert!(
                matches!(result.unwrap().output, RuntimeCommandOutput::Delay(ms) if ms >= 2_000)
            );
        } else {
            assert_eq!(result.unwrap_err().code, ErrorCode::RequestTimeout);
        }
        jobs -= 1;
    }
    server.shutdown();
    backend.shutdown();
}

#[test]
fn proxy_groups_follow_merged_config_order() {
    let (_dir, config) = fixture();
    let mut backend = Backend::new(config).unwrap();
    let id = ProfileId::parse("order").unwrap();
    backend.profiles.import(Profile::new(id.clone(), "Order", ProfileSource::Local, UpdatePolicy::Manual, 0, None).unwrap(),
        "proxy-groups:\n  - name: Z\n    type: select\n    proxies: [DIRECT]\n  - name: A\n    type: select\n    proxies: [DIRECT]\n").unwrap();
    backend.profiles.select(&id).unwrap();
    let mut response = UiResponse::Runtime {
        request: RuntimeCommand::ListProxyGroups,
        result: Ok(crate::domain::RuntimeCommandResult {
            output: RuntimeCommandOutput::ProxyGroups(crate::domain::ProxySnapshot {
                groups: ["GLOBAL", "A", "Z"]
                    .map(|name| crate::domain::ProxyGroup {
                        name: name.into(),
                        kind: "Selector".into(),
                        selected: None,
                        members: vec![],
                    })
                    .into(),
                ..Default::default()
            }),
            summary: String::new(),
        }),
    };
    backend.order_proxy_groups(&mut response);
    let UiResponse::Runtime {
        result: Ok(result), ..
    } = response
    else {
        unreachable!()
    };
    let RuntimeCommandOutput::ProxyGroups(snapshot) = result.output else {
        unreachable!()
    };
    assert_eq!(
        snapshot
            .groups
            .iter()
            .map(|g| g.name.as_str())
            .collect::<Vec<_>>(),
        ["Z", "A", "GLOBAL"]
    );
}
