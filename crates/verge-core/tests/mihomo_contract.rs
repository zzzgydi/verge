use std::{
    env, fs,
    io::Write,
    net::{SocketAddr, TcpListener},
    path::{Path, PathBuf},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use verge_core::{
    CoreSupervisor, MihomoClient, MihomoConfig, RealtimeEvent, RealtimeOptions,
    RealtimeSubscription, RealtimeTopic, RunMode, SidecarManifest, TcpControllerTransport,
};
use verge_domain::ErrorCode;

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "verge-mihomo-contract-{}-{nonce}",
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

fn available_controller() -> SocketAddr {
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

fn wait_for_event(
    name: &str,
    subscription: &RealtimeSubscription,
    matches: impl Fn(&RealtimeEvent) -> bool,
) -> RealtimeEvent {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let mut observed = Vec::new();
    while std::time::Instant::now() < deadline {
        for event in subscription.drain() {
            if matches(&event) {
                return event;
            }
            observed.push(event);
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("Mihomo {name} event did not arrive; observed: {observed:?}");
}

#[test]
#[ignore = "requires MIHOMO_BIN pointing to the pinned sidecar"]
fn pinned_mihomo_process_and_rest_contract() {
    let binary = PathBuf::from(env::var_os("MIHOMO_BIN").expect("MIHOMO_BIN is required"));
    let manifest = SidecarManifest::load(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/mihomo/manifest.json"),
    )
    .unwrap();
    assert_eq!(manifest.version, "1.19.26");
    manifest
        .target("aarch64-apple-darwin")
        .unwrap()
        .verify_executable(&binary)
        .unwrap();
    let directory = TestDir::new();
    let controller = available_controller();
    let mixed_port = available_controller().port();
    let secret = "verge-contract-secret";
    let config_path = directory.0.join("config.yaml");
    write_config(&config_path, controller, mixed_port, secret);

    let settings = MihomoConfig {
        binary,
        working_dir: directory.0.clone(),
        controller,
        secret: secret.into(),
        max_restarts: 1,
        restart_backoff: Duration::from_millis(10),
        stop_timeout: Duration::from_secs(2),
        log_capacity: 100,
    };
    let mut supervisor = CoreSupervisor::new(settings, config_path).unwrap();
    let invalid = directory.0.join("invalid.yaml");
    fs::write(&invalid, "- invalid-root\n").unwrap();
    assert_eq!(
        supervisor.validate_candidate(&invalid).unwrap_err().code,
        ErrorCode::CoreRejectedConfig
    );
    supervisor.start().unwrap();

    let transport =
        TcpControllerTransport::new(controller, secret, Duration::from_secs(1)).unwrap();
    let mut client = MihomoClient::new(transport);
    let version = (0..50)
        .find_map(|_| match client.version() {
            Ok(version) => Some(version),
            Err(_) => {
                thread::sleep(Duration::from_millis(20));
                None
            }
        })
        .expect("Mihomo controller did not become ready");
    assert_eq!(version, "v1.19.26");

    let realtime_options = RealtimeOptions {
        connect_timeout: Duration::from_secs(1),
        read_timeout: Duration::from_millis(50),
        initial_backoff: Duration::from_millis(20),
        max_backoff: Duration::from_millis(100),
        buffer_capacity: 32,
    };
    let mut traffic = RealtimeSubscription::spawn(
        controller,
        secret,
        RealtimeTopic::Traffic,
        realtime_options.clone(),
    )
    .unwrap();
    let mut memory = RealtimeSubscription::spawn(
        controller,
        secret,
        RealtimeTopic::Memory,
        realtime_options.clone(),
    )
    .unwrap();
    let mut connections = RealtimeSubscription::spawn(
        controller,
        secret,
        RealtimeTopic::Connections,
        realtime_options.clone(),
    )
    .unwrap();
    let mut logs =
        RealtimeSubscription::spawn(controller, secret, RealtimeTopic::Logs, realtime_options)
            .unwrap();

    assert_eq!(client.mode().unwrap(), RunMode::Rule);
    client.set_mode(RunMode::Global).unwrap();
    assert_eq!(client.mode().unwrap(), RunMode::Global);
    client.set_mode(RunMode::Rule).unwrap();
    thread::sleep(Duration::from_millis(100));
    let mut proxy = std::net::TcpStream::connect(("127.0.0.1", mixed_port)).unwrap();
    proxy
        .write_all(b"GET http://example.invalid/ HTTP/1.1\r\nHost: example.invalid\r\n\r\n")
        .unwrap();

    wait_for_event("traffic", &traffic, |event| {
        matches!(event, RealtimeEvent::Traffic(_))
    });
    wait_for_event("memory", &memory, |event| {
        matches!(event, RealtimeEvent::Memory(_))
    });
    wait_for_event("connections", &connections, |event| {
        matches!(event, RealtimeEvent::Connections(_))
    });
    wait_for_event("logs", &logs, |event| {
        matches!(event, RealtimeEvent::Log(_))
    });

    traffic.stop();
    memory.stop();
    connections.stop();
    logs.stop();

    supervisor.stop().unwrap();
}
