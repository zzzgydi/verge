//! Isolated contract: no system proxy, helper, TUN, subscriptions, or real traffic.
use std::{
    env, fs,
    net::{SocketAddr, TcpListener},
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
use verge::{
    application::{ConfigCommandHandler, RuntimeCredentials, SupervisorControl},
    config::FileProfileStore,
    domain::{
        CoreNetworkSettings, Profile, ProfileId, ProfileSource, RealtimeEvent, RealtimeTopic,
        UpdatePolicy,
    },
    mihomo::{
        ControllerEndpoint, CoreSupervisor, MihomoClient, MihomoConfig, RealtimeOptions,
        RealtimeSubscription, SidecarManifest, TcpControllerTransport,
    },
};
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn address() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

#[test]
#[ignore = "requires MIHOMO_BIN pointing to the pinned sidecar"]
fn runtime_network_overrides_and_independent_internal_controller() {
    let binary = PathBuf::from(env::var_os("MIHOMO_BIN").expect("MIHOMO_BIN"));
    SidecarManifest::load(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/mihomo/manifest.json"),
    )
    .unwrap()
    .target("aarch64-apple-darwin")
    .unwrap()
    .verify_executable(&binary)
    .unwrap();
    let dir = Temp(PathBuf::from(format!(
        "/tmp/verge-net-{}",
        std::process::id()
    )));
    fs::create_dir_all(&dir.0).unwrap();
    let socket = dir.0.join("control.sock");
    let mut profiles = FileProfileStore::open(dir.0.join("profiles")).unwrap();
    profiles.set_internal_socket(socket.clone());
    let id = ProfileId::parse("contract").unwrap();
    let source = "mixed-port: 0\nmode: rule\nlog-level: warning\nrules: ['MATCH,DIRECT']\ndns:\n  enable: false\n";
    profiles
        .import(
            Profile::new(
                id.clone(),
                "Contract",
                ProfileSource::Local,
                UpdatePolicy::Manual,
                0,
                None,
            )
            .unwrap(),
            source,
        )
        .unwrap();
    profiles.select(&id).unwrap();
    let mut settings = CoreNetworkSettings {
        mixed_port: address().port(),
        ..Default::default()
    };
    settings.dns_yaml = "enable: false\nnameserver: [1.1.1.1]\n".into();
    profiles
        .set_network_override(Some(settings.clone()))
        .unwrap();
    let credentials = RuntimeCredentials {
        controller: address(),
        secret: "test-internal".into(),
    };
    let path = profiles
        .materialize_runtime(&id, credentials.controller, &credentials.secret)
        .unwrap();
    let mut supervisor = CoreSupervisor::new(
        MihomoConfig {
            binary,
            working_dir: dir.0.clone(),
            controller: credentials.controller,
            secret: credentials.secret.clone(),
            max_restarts: 1,
            restart_backoff: Duration::from_millis(20),
            stop_timeout: Duration::from_secs(2),
            log_capacity: 100,
        },
        path.clone(),
    )
    .unwrap();
    supervisor.set_internal_socket(socket.clone());
    supervisor.validate_candidate(&path).unwrap();
    supervisor.start().unwrap();
    let ready = |supervisor: &CoreSupervisor| {
        let end = Instant::now() + Duration::from_secs(4);
        while supervisor.health(Duration::from_millis(100)).is_err() {
            assert!(
                Instant::now() < end,
                "controller unavailable: {:?}",
                supervisor.logs()
            );
            thread::sleep(Duration::from_millis(20));
        }
    };
    ready(&supervisor);
    assert!(std::net::TcpStream::connect(credentials.controller).is_err());
    let mut client = MihomoClient::new(
        TcpControllerTransport::unix(socket.clone(), Duration::from_secs(1)).unwrap(),
    );
    assert!(client.version().is_ok());
    let mut subscription = RealtimeSubscription::spawn(
        ControllerEndpoint::Unix(socket.clone()),
        "",
        RealtimeTopic::Traffic,
        RealtimeOptions::default(),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !subscription
        .drain()
        .iter()
        .any(|event| matches!(event, RealtimeEvent::Traffic(_)))
    {
        assert!(
            Instant::now() < deadline,
            "Unix WebSocket produced no traffic event"
        );
        thread::sleep(Duration::from_millis(50));
    }
    subscription.stop();
    let external = address();
    settings.external_controller.enabled = true;
    settings.external_controller.address = external.to_string();
    settings.external_controller.secret = "external-test-one".into();
    let apply = |profiles: &mut FileProfileStore,
                 supervisor: &mut CoreSupervisor,
                 settings: CoreNetworkSettings| {
        let mut core = SupervisorControl::new(supervisor, Duration::from_secs(4));
        ConfigCommandHandler::with_runtime_credentials(profiles, &mut core, credentials.clone())
            .update_network_settings(settings, |_| Ok(()))
    };
    apply(&mut profiles, &mut supervisor, settings.clone()).unwrap();
    assert!(
        MihomoClient::new(
            TcpControllerTransport::new(external, "wrong", Duration::from_secs(1)).unwrap()
        )
        .version()
        .is_err()
    );
    assert!(
        MihomoClient::new(
            TcpControllerTransport::new(external, "external-test-one", Duration::from_secs(1))
                .unwrap()
        )
        .version()
        .is_ok()
    );
    settings.external_controller.secret = "external-test-two".into();
    settings.mixed_port = address().port();
    settings.ipv6 = true;
    settings.dns_override = true;
    apply(&mut profiles, &mut supervisor, settings.clone()).unwrap();
    assert!(client.network_settings().unwrap().ipv6_enabled);
    assert!(std::net::TcpStream::connect(("127.0.0.1", settings.mixed_port)).is_ok());
    assert!(
        MihomoClient::new(
            TcpControllerTransport::new(external, "external-test-one", Duration::from_secs(1))
                .unwrap()
        )
        .version()
        .is_err()
    );
    settings.external_controller.enabled = false;
    apply(&mut profiles, &mut supervisor, settings.clone()).unwrap();
    assert!(std::net::TcpStream::connect(external).is_err());
    assert!(client.version().is_ok());
    let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut conflicting = settings.clone();
    conflicting.external_controller.enabled = true;
    conflicting.external_controller.address = occupied.local_addr().unwrap().to_string();
    assert!(apply(&mut profiles, &mut supervisor, conflicting).is_err());
    assert_eq!(profiles.network_settings().unwrap(), settings);
    assert!(client.version().is_ok());
    let mut busy_proxy = settings.clone();
    busy_proxy.mixed_port = occupied.local_addr().unwrap().port();
    assert!(apply(&mut profiles, &mut supervisor, busy_proxy).is_err());
    assert_eq!(profiles.network_settings().unwrap(), settings);
    let second = ProfileId::parse("second").unwrap();
    profiles
        .import(
            Profile::new(
                second.clone(),
                "Second",
                ProfileSource::Local,
                UpdatePolicy::Manual,
                0,
                None,
            )
            .unwrap(),
            "mixed-port: 12345\nmode: direct\nrules: ['MATCH,DIRECT']\n",
        )
        .unwrap();
    verge::application::ProfileCommandHandler::new(
        &mut profiles,
        &mut SupervisorControl::new(&mut supervisor, Duration::from_secs(4)),
        Some(credentials.clone()),
    )
    .execute(
        verge::domain::AppCommand::SelectProfile { id: second.clone() },
        1,
    )
    .unwrap();
    assert_eq!(
        profiles.system_proxy_endpoint(&second).unwrap().port,
        settings.mixed_port
    );
    assert!(client.network_settings().unwrap().ipv6_enabled);
    let mut invalid = settings.clone();
    invalid.dns_yaml = "enable: true\nnameserver: []\n".into();
    assert!(apply(&mut profiles, &mut supervisor, invalid).is_err());
    assert_eq!(profiles.network_settings().unwrap(), settings);
    assert!(client.version().is_ok());
    assert_eq!(profiles.yaml(&id).unwrap(), source);
    supervisor.stop().unwrap();
}
