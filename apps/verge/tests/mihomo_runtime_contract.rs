use std::{
    env, fs,
    io::Write,
    net::{SocketAddr, TcpListener},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use verge::application::{MihomoRuntime, RuntimeCommandHandler};
use verge::mihomo::{
    CoreSupervisor, MihomoClient, MihomoConfig, RealtimeOptions, SidecarManifest,
    TcpControllerTransport,
};
use verge::domain::{RealtimeEvent, RealtimeTopic, RunMode, RuntimeCommand, RuntimeCommandOutput};

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "verge-runtime-contract-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn available_address() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap()
}

fn write_config(path: &Path, controller: SocketAddr, mixed_port: u16, secret: &str) {
    fs::write(
        path,
        format!(
            "mixed-port: {mixed_port}\nexternal-controller: {controller}\nsecret: {secret}\nmode: rule\nlog-level: debug\nipv6: false\nproxies:\n  - name: dead-local\n    type: http\n    server: 127.0.0.1\n    port: 9\nproxy-groups:\n  - name: contract\n    type: select\n    proxies: [dead-local, DIRECT]\nrules:\n  - MATCH,contract\n"
        ),
    )
    .unwrap();
}

#[test]
#[ignore = "requires MIHOMO_BIN pointing to the pinned sidecar"]
fn application_commands_drive_pinned_mihomo() {
    let binary = PathBuf::from(env::var_os("MIHOMO_BIN").expect("MIHOMO_BIN is required"));
    SidecarManifest::load(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/mihomo/manifest.json"),
    )
    .unwrap()
    .target("aarch64-apple-darwin")
    .unwrap()
    .verify_executable(&binary)
    .unwrap();

    let directory = TestDir::new();
    let controller = available_address();
    let mixed_port = available_address().port();
    let secret = "verge-application-contract-secret";
    let config_path = directory.0.join("config.yaml");
    write_config(&config_path, controller, mixed_port, secret);
    let mut supervisor = CoreSupervisor::new(
        MihomoConfig {
            binary,
            working_dir: directory.0.clone(),
            controller,
            secret: secret.into(),
            max_restarts: 1,
            restart_backoff: Duration::from_millis(10),
            stop_timeout: Duration::from_secs(2),
            log_capacity: 100,
        },
        config_path,
    )
    .unwrap();
    supervisor.start().unwrap();
    (0..50)
        .find(|_| {
            if supervisor.health(Duration::from_millis(100)).is_ok() {
                true
            } else {
                thread::sleep(Duration::from_millis(20));
                false
            }
        })
        .expect("Mihomo controller did not become ready");

    let transport =
        TcpControllerTransport::new(controller, secret, Duration::from_secs(1)).unwrap();
    let mut runtime = MihomoRuntime::new(
        MihomoClient::new(transport),
        controller,
        secret,
        RealtimeOptions {
            connect_timeout: Duration::from_secs(1),
            read_timeout: Duration::from_millis(50),
            initial_backoff: Duration::from_millis(20),
            max_backoff: Duration::from_millis(100),
            buffer_capacity: 32,
        },
    )
    .unwrap();
    let mut handler = RuntimeCommandHandler::new(&mut runtime);

    assert_eq!(
        handler.execute(RuntimeCommand::GetMode).unwrap().output,
        RuntimeCommandOutput::Mode(RunMode::Rule)
    );
    handler
        .execute(RuntimeCommand::SetMode {
            mode: RunMode::Global,
        })
        .unwrap();
    assert!(matches!(
        handler
            .execute(RuntimeCommand::ListProxyGroups)
            .unwrap()
            .output,
        RuntimeCommandOutput::ProxyGroups(groups)
            if groups.iter().any(|group| group.name == "contract")
    ));
    handler
        .execute(RuntimeCommand::SelectProxy {
            group: "contract".into(),
            proxy: "dead-local".into(),
        })
        .unwrap();
    handler
        .execute(RuntimeCommand::SetMode {
            mode: RunMode::Rule,
        })
        .unwrap();
    handler
        .execute(RuntimeCommand::StartRealtime {
            topics: vec![
                RealtimeTopic::Traffic,
                RealtimeTopic::Memory,
                RealtimeTopic::Connections,
                RealtimeTopic::Logs,
            ],
        })
        .unwrap();
    thread::sleep(Duration::from_millis(100));
    let mut proxy = std::net::TcpStream::connect(("127.0.0.1", mixed_port)).unwrap();
    proxy
        .write_all(b"GET http://example.invalid/ HTTP/1.1\r\nHost: example.invalid\r\n\r\n")
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut traffic = false;
    let mut memory = false;
    let mut connections = false;
    let mut logs = false;
    while Instant::now() < deadline && !(traffic && memory && connections && logs) {
        let result = handler.execute(RuntimeCommand::DrainRealtime).unwrap();
        if let RuntimeCommandOutput::Realtime(events) = result.output {
            for event in events {
                traffic |= matches!(event, RealtimeEvent::Traffic(_));
                memory |= matches!(event, RealtimeEvent::Memory(_));
                connections |= matches!(event, RealtimeEvent::Connections(_));
                logs |= matches!(event, RealtimeEvent::Log(_));
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    assert!((traffic && memory && connections && logs));
    handler.execute(RuntimeCommand::StopRealtime).unwrap();
    drop(runtime);
    supervisor.stop().unwrap();
}
