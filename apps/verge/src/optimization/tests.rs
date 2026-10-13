use super::*;
use crate::{
    domain::{ProxyDetails, ProxyGroup, ProxySnapshot, RunMode},
    mihomo::{ControllerRequest, ControllerResponse, ControllerTransport, MihomoClient},
};
use std::{
    cell::Cell,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64},
    },
    time::{Duration, Instant},
};

fn proxies() -> ProxySnapshot {
    ProxySnapshot {
        groups: vec![ProxyGroup {
            name: "group".into(),
            kind: "Selector".into(),
            selected: Some("a".into()),
            members: vec!["a".into(), "b".into(), "c".into(), "DIRECT".into()],
        }],
        proxies: ["a", "b", "c", "DIRECT"]
            .into_iter()
            .map(|name| {
                (
                    name.into(),
                    ProxyDetails {
                        kind: if name == "DIRECT" {
                            "Direct"
                        } else {
                            "Shadowsocks"
                        }
                        .into(),
                        ..Default::default()
                    },
                )
            })
            .collect(),
    }
}

fn baseline(proxies: &ProxySnapshot, mode: RunMode) -> Result<String, AppError> {
    Ok(format!("{:?}:{mode:?}", proxies.groups))
}

fn pending() -> checks::Pending {
    checks::Pending {
        plan: Plan {
            id: "opaque".into(),
            group: "group".into(),
            previous: "a".into(),
            proposed: "b".into(),
            expires_at: now() + 120,
        },
        baseline: baseline(&proxies(), RunMode::Rule).unwrap(),
        created: Instant::now(),
    }
}

#[derive(Default)]
struct FakeState {
    selected: String,
    writes: Vec<String>,
    probes: usize,
    fail_probe: bool,
    ignore_write: bool,
    lost_write_response: bool,
    external_change: bool,
}

struct Fake(Arc<Mutex<FakeState>>);
impl ControllerTransport for Fake {
    fn send(&mut self, request: ControllerRequest) -> Result<ControllerResponse, AppError> {
        let mut state = self.0.lock().unwrap();
        let mut status = 200;
        let body = match (request.method, request.path.as_str()) {
            ("GET", "/configs") => serde_json::json!({"mode":"rule"}),
            ("GET", "/proxies") => {
                if state.external_change && state.probes > 0 {
                    state.selected = "c".into();
                }
                serde_json::json!({"proxies":{
                    "group":{"type":"Selector","now":state.selected,"all":["a","b","c","DIRECT"]},
                    "a":{"type":"Shadowsocks"},"b":{"type":"Shadowsocks"},"c":{"type":"Shadowsocks"},"DIRECT":{"type":"Direct"}
                }})
            }
            ("PUT", "/proxies/group") => {
                let node = serde_json::from_str::<serde_json::Value>(&request.body.unwrap())
                    .unwrap()["name"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                state.writes.push(node.clone());
                if !state.ignore_write {
                    state.selected = node;
                }
                if state.lost_write_response {
                    return Err(error("lost response"));
                }
                status = 204;
                serde_json::Value::Null
            }
            ("GET", path) if path.contains("/delay?") => {
                state.probes += 1;
                if state.fail_probe {
                    return Err(error("timeout"));
                }
                serde_json::json!({"delay":20})
            }
            _ => panic!("unexpected request: {request:?}"),
        };
        Ok(ControllerResponse {
            status,
            body: body.to_string(),
        })
    }
}

fn client(state: FakeState) -> (MihomoClient<Fake>, Arc<Mutex<FakeState>>) {
    let state = Arc::new(Mutex::new(FakeState {
        selected: "a".into(),
        ..state
    }));
    (MihomoClient::new(Fake(state.clone())), state)
}

struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new() -> Self {
        let mut id = [0; 8];
        getrandom::fill(&mut id).unwrap();
        let path = std::path::PathBuf::from("/tmp")
            .join(format!("verge-optimization-{}", hex::encode(id)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn context(&self) -> Context {
        Context {
            socket: self.0.join("missing.sock"),
            profiles: crate::config::FileProfileStore::open(self.0.clone()).unwrap(),
            services: Vec::new(),
            recovery: self.0.join("recovery.json"),
            target: None,
            tun_connected: false,
            errors: None,
            connections: None,
            epoch: Arc::new(AtomicU64::new(0)),
            active_probes: false,
            sample_cursor: 0,
        }
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn model_evidence_never_contains_names_or_log_details() {
    let report = Report {
        id: 7,
        findings: vec![Finding {
            code: "logs".into(),
            severity: Severity::Warning,
            detail: "private-host.example".into(),
        }],
        measurements: vec![Measurement {
            node: "secret-node".into(),
            target: 0,
            delay_ms: None,
            captured_at: 1,
        }],
        groups: vec!["private-group".into()],
        ..Default::default()
    };
    let evidence = serde_json::to_string(&model_evidence(&report)).unwrap();
    for private in ["private-host", "secret-node", "private-group"] {
        assert!(!evidence.contains(private));
    }
    assert!(evidence.contains("logs"));
}

#[test]
fn health_requires_both_targets_on_the_same_node() {
    let mut report = Report::default();
    assert!(!checks::healthy(&report, "a"));
    report.measurements.push(Measurement {
        node: "a".into(),
        target: 0,
        delay_ms: Some(10),
        captured_at: 1,
    });
    report.measurements.push(Measurement {
        node: "b".into(),
        target: 1,
        delay_ms: Some(10),
        captured_at: 1,
    });
    assert!(!checks::healthy(&report, "a"));
    report.measurements.push(Measurement {
        node: "a".into(),
        target: 1,
        delay_ms: Some(10),
        captured_at: 1,
    });
    assert!(checks::healthy(&report, "a"));
}

#[test]
fn traversal_rejects_cycles_direct_and_nested_switches() {
    let mut snapshot = proxies();
    assert_eq!(checks::leaf(&snapshot, "group"), Some("a".into()));
    assert!(!checks::eligible(&snapshot, "DIRECT"));
    assert!(checks::selected_group(&snapshot, "group").is_some());
    snapshot.groups[0].selected = Some("group".into());
    assert_eq!(checks::leaf(&snapshot, "group"), None);
    assert!(checks::selected_group(&snapshot, "group").is_none());
    snapshot.groups[0].selected = Some("missing".into());
    assert_eq!(checks::leaf(&snapshot, "group"), None);
}

#[test]
fn isolated_failure_is_not_a_switch_signal() {
    let mut report = Report::default();
    for target in 0..2 {
        report.measurements.push(Measurement {
            node: "a".into(),
            target,
            delay_ms: None,
            captured_at: 0,
        });
    }
    assert!(!checks::failed_twice(&report, "a"));
    report.measurements.extend(report.measurements.clone());
    assert!(checks::failed_twice(&report, "a"));
}

#[test]
fn apply_requires_durable_intent_and_verifies_both_targets() {
    let (mut client, state) = client(FakeState::default());
    let intent = Cell::new(false);
    let mut report = Report::default();
    checks::apply_verified(&mut client, &pending(), &mut report, baseline, || {
        assert!(state.lock().unwrap().writes.is_empty());
        intent.set(true);
        Ok(())
    })
    .unwrap();
    assert!(intent.get());
    assert_eq!(report.outcome.as_deref(), Some("optimization.applied"));
    assert_eq!(state.lock().unwrap().writes, ["b"]);
    assert_eq!(state.lock().unwrap().probes, 2);
}

#[test]
fn intent_failure_and_stale_or_expired_plans_never_write() {
    let (mut client, state) = client(FakeState::default());
    let mut report = Report::default();
    assert!(
        checks::apply_verified(&mut client, &pending(), &mut report, baseline, || Err(
            error("disk full")
        ))
        .is_err()
    );
    let mut stale = pending();
    stale.baseline = "old".into();
    assert!(
        checks::apply_verified(&mut client, &stale, &mut report, baseline, || panic!(
            "stale intent"
        ))
        .is_err()
    );
    let mut expired = pending();
    expired.created = Instant::now() - Duration::from_secs(121);
    assert!(
        checks::apply_verified(&mut client, &expired, &mut report, baseline, || panic!(
            "expired intent"
        ))
        .is_err()
    );
    assert!(state.lock().unwrap().writes.is_empty());
}

#[test]
fn failed_verification_rolls_back_even_if_write_responses_are_lost() {
    let (mut client, state) = client(FakeState {
        fail_probe: true,
        lost_write_response: true,
        ..Default::default()
    });
    let mut report = Report::default();
    checks::apply_verified(&mut client, &pending(), &mut report, baseline, || Ok(())).unwrap();
    assert_eq!(state.lock().unwrap().writes, ["b", "a"]);
    assert_eq!(report.outcome.as_deref(), Some("optimization.restored"));
}

#[test]
fn rollback_never_overwrites_an_external_selection() {
    let (mut client, state) = client(FakeState {
        fail_probe: true,
        external_change: true,
        ..Default::default()
    });
    assert!(
        checks::apply_verified(
            &mut client,
            &pending(),
            &mut Report::default(),
            baseline,
            || Ok(())
        )
        .is_err()
    );
    assert_eq!(state.lock().unwrap().writes, ["b"]);
    assert_eq!(state.lock().unwrap().selected, "c");
}

#[test]
fn rejected_write_does_not_claim_success_or_attempt_rollback() {
    let (mut client, state) = client(FakeState {
        ignore_write: true,
        ..Default::default()
    });
    let mut report = Report::default();
    checks::apply_verified(&mut client, &pending(), &mut report, baseline, || Ok(())).unwrap();
    assert_eq!(report.outcome.as_deref(), Some("optimization.not_applied"));
    assert_eq!(state.lock().unwrap().probes, 0);
}

#[test]
fn cancelled_and_deadline_limited_probes_do_not_send_requests() {
    let (mut client, state) = client(FakeState::default());
    let mut report = Report::default();
    checks::measure(
        &mut client,
        "a",
        &mut report,
        &AtomicBool::new(true),
        Instant::now() + Duration::from_secs(30),
    );
    assert!(report.partial);
    checks::measure(
        &mut client,
        "a",
        &mut report,
        &AtomicBool::new(false),
        Instant::now(),
    );
    assert_eq!(state.lock().unwrap().probes, 0);
}

#[test]
fn history_is_bounded_redacted_and_non_replayable() {
    let report = Report {
        captured_at: now(),
        groups: vec!["private-group".into()],
        plan: Some(pending().plan),
        explanation: Some("cloud answer".into()),
        measurements: vec![Measurement {
            node: "private-node".into(),
            target: 0,
            delay_ms: Some(10),
            captured_at: now(),
        }],
        ..Default::default()
    };
    let saved = service::persisted_history(&Snapshot {
        history: vec![report; 40],
        ..Default::default()
    });
    assert_eq!(saved.len(), 24);
    let json = serde_json::to_string(&saved).unwrap();
    for private in ["private-", "opaque", "cloud answer"] {
        assert!(!json.contains(private));
    }
    assert!(json.contains("node-1"));
}

#[test]
fn default_is_off_and_interrupted_intent_is_not_replayed() {
    let directory = Scratch::new();
    let mut service = Service::new(directory.0.clone());
    assert!(!service.snapshot().observe);
    service.observe(&crate::domain::RealtimeEvent::Log(
        crate::domain::LogEvent {
            level: "error".into(),
            payload: "private".into(),
        },
    ));
    assert_eq!(service.observations(), (None, None));
    assert!(
        service
            .handle(
                Command::Apply {
                    id: "invented".into()
                },
                directory.context()
            )
            .is_err()
    );
    std::fs::write(directory.0.join("optimization-intent.json"), "{}").unwrap();
    let mut restarted = Service::new(directory.0.clone());
    assert_eq!(
        restarted.snapshot().error.as_deref(),
        Some("optimization.apply_unknown")
    );
    restarted
        .handle(Command::ClearHistory, directory.context())
        .unwrap();
    assert!(!directory.0.join("optimization-intent.json").exists());
}

#[test]
fn daily_budget_survives_restart_and_clear() {
    let directory = Scratch::new();
    std::fs::write(
        directory.0.join("optimization.json"),
        serde_json::json!({"observe":true,"day":now()/86400,"runs":144,"analyses":10}).to_string(),
    )
    .unwrap();
    let mut service = Service::new(directory.0.clone());
    service
        .handle(Command::ClearHistory, directory.context())
        .unwrap();
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.0.join("optimization.json")).unwrap())
            .unwrap();
    assert_eq!(saved["runs"], 144);
    assert_eq!(saved["analyses"], 10);
    assert!(service.snapshot().observe);
    assert!(
        !service.snapshot().active_probes,
        "old observation consent must not enable probes"
    );
}

#[test]
fn active_probe_consent_is_required_persisted_and_revocable() {
    let directory = Scratch::new();
    let mut service = Service::new(directory.0.clone());
    // Even a caller-supplied Context cannot bypass the daemon-owned permission.
    let mut context = directory.context();
    context.active_probes = true;
    for command in [
        Command::Optimize {
            group: "group".into(),
        },
        Command::Apply { id: "fake".into() },
    ] {
        assert_eq!(
            service
                .handle(command, context.clone())
                .unwrap_err()
                .message,
            "optimization.probes_required"
        );
    }
    service
        .handle(Command::SetActiveProbes { enabled: true }, context.clone())
        .unwrap();
    let mut restarted = Service::new(directory.0.clone());
    assert!(restarted.snapshot().active_probes);
    restarted
        .handle(Command::ClearHistory, context.clone())
        .unwrap();
    assert!(restarted.snapshot().active_probes);
    restarted
        .handle(Command::SetActiveProbes { enabled: false }, context.clone())
        .unwrap();
    assert!(!Service::new(directory.0.clone()).snapshot().active_probes);
    assert_eq!(
        restarted
            .handle(
                Command::Optimize {
                    group: "group".into()
                },
                context
            )
            .unwrap_err()
            .message,
        "optimization.probes_required"
    );
    assert!(
        checks::apply(
            &directory.context(),
            &pending(),
            &mut Report::default(),
            || panic!("no journal without consent")
        )
        .is_err()
    );
}

#[test]
fn bounded_sampling_rotates_and_handles_empty_or_changed_topology() {
    let nodes = ["a", "b", "c", "d", "e"].map(String::from);
    for cursor in 0..10 {
        assert_eq!(
            checks::sampled_nodes(&nodes, cursor, 1),
            vec![&nodes[cursor as usize % 5]]
        );
        let manual = checks::sampled_nodes(&nodes, cursor, 3);
        assert_eq!(manual.len(), 3);
        assert_eq!(manual[0], &nodes[cursor as usize % 5]);
    }
    assert!(checks::sampled_nodes(&[], u64::MAX, 1).is_empty());
    assert_eq!(
        checks::sampled_nodes(&nodes[..1], u64::MAX, 3),
        vec![&nodes[0]]
    );
    assert_eq!(
        checks::sampled_nodes(&nodes[..2], 4, 3),
        vec![&nodes[0], &nodes[1]]
    );
}

#[test]
fn local_checks_never_send_delay_requests_without_consent_and_rotate_when_allowed() {
    use std::{
        io::{Read, Write},
        os::unix::net::UnixListener,
    };
    let directory = Scratch::new();
    let context = directory.context();
    let listener = UnixListener::bind(&context.socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let server = std::thread::spawn(move || {
        while !stopped.load(std::sync::atomic::Ordering::Relaxed) {
            let Ok((mut stream, _)) = listener.accept() else {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut byte = [0];
            while !bytes.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                bytes.push(byte[0]);
            }
            let text = String::from_utf8(bytes).unwrap();
            let path = text.split_whitespace().nth(1).unwrap();
            captured.lock().unwrap().push(path.to_owned());
            let body = match path {
                "/version" => serde_json::json!({"version":"test"}),
                "/configs" => serde_json::json!({"mode":"rule"}),
                "/proxies" => serde_json::json!({"proxies":{
                    "GroupA":{"type":"Selector","now":"a","all":["a","b"]},
                    "GroupB":{"type":"Selector","now":"b","all":["a","b"]},
                    "a":{"type":"Shadowsocks"}, "b":{"type":"Shadowsocks"}
                }}),
                _ if path.contains("/delay?") => serde_json::json!({"delay":20}),
                _ => panic!("unexpected request: {path}"),
            }
            .to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        }
    });
    for background in [false, true] {
        let report = checks::check(
            &context,
            1,
            None,
            background,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap()
        .report;
        assert!(report.measurements.is_empty());
        assert!(report.findings.iter().any(|f| f.code == "probes_disabled"));
    }
    assert!(
        requests
            .lock()
            .unwrap()
            .iter()
            .all(|p| !p.contains("/delay?"))
    );
    let mut context = context;
    context.active_probes = true;
    for (cursor, node) in [(0, "a"), (1, "b"), (2, "a")] {
        context.sample_cursor = cursor;
        let report = checks::check(
            &context,
            cursor + 2,
            None,
            true,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap()
        .report;
        assert_eq!(report.measurements.len(), 2);
        assert!(report.measurements.iter().all(|m| m.node == node));
        assert!(report.partial);
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    server.join().unwrap();
}

#[test]
fn warning_dial_failures_reach_observer_without_retaining_payloads() {
    let directory = Scratch::new();
    let mut service = Service::new(directory.0.clone());
    service
        .handle(
            Command::Configure {
                observe: true,
                model_analysis: false,
            },
            directory.context(),
        )
        .unwrap();
    for (level, payload) in [
        (
            "warning",
            "[TCP] dial Proxy (match Match/) 127.0.0.1:1 --> private.test:443 error: connect: connection refused",
        ),
        (
            "warn",
            "[UDP] dial Proxy 127.0.0.1:2 --> private.test:53 error: dns resolve failed",
        ),
        ("ERROR", "core failure"),
        ("warning", "configuration uses a deprecated field"),
        ("info", "[TCP] dial Proxy error: not a failure-level event"),
    ] {
        service.observe(&crate::domain::RealtimeEvent::Log(
            crate::domain::LogEvent {
                level: level.into(),
                payload: payload.into(),
            },
        ));
    }
    assert_eq!(service.observations().0, Some(3));
    let saved = std::fs::read_to_string(directory.0.join("optimization.json")).unwrap();
    assert!(!saved.contains("private.test"));
    service
        .handle(
            Command::Configure {
                observe: false,
                model_analysis: false,
            },
            directory.context(),
        )
        .unwrap();
    assert_eq!(service.observations().0, None);
}

#[test]
fn local_check_with_no_core_finishes_without_any_model() {
    let directory = Scratch::new();
    let result = checks::check(
        &directory.context(),
        1,
        None,
        false,
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap();
    assert!(result.report.partial);
    assert!(
        result
            .report
            .findings
            .iter()
            .any(|f| f.code == "core" && f.severity == Severity::Warning)
    );
    assert!(result.pending.is_none());
}

#[test]
fn configuration_or_network_change_during_confirmation_prevents_write() {
    let (mut client, state) = client(FakeState::default());
    let calls = Cell::new(0);
    let result = checks::apply_verified(
        &mut client,
        &pending(),
        &mut Report::default(),
        |proxies, mode| {
            calls.set(calls.get() + 1);
            if calls.get() >= 2 {
                Ok("changed-network".into())
            } else {
                baseline(proxies, mode)
            }
        },
        || panic!("must not journal a stale plan"),
    );
    assert!(result.is_err());
    assert!(state.lock().unwrap().writes.is_empty());
}

#[test]
#[ignore = "requires MIHOMO_BIN pointing to the pinned sidecar"]
fn optimization_failed_probe_restores_selection_on_pinned_mihomo() {
    use crate::mihomo::{CoreSupervisor, MihomoConfig, SidecarManifest};
    let binary = std::path::PathBuf::from(std::env::var_os("MIHOMO_BIN").expect("MIHOMO_BIN"));
    SidecarManifest::load(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/mihomo/manifest.json"),
    )
    .unwrap()
    .target("aarch64-apple-darwin")
    .unwrap()
    .verify_executable(&binary)
    .unwrap();
    let directory = Scratch::new();
    let mut context = directory.context();
    context.active_probes = true;
    context.socket = directory.0.join("core.sock");
    let controller = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let dead_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config_path = directory.0.join("test.yaml");
    std::fs::write(&config_path, format!("external-controller-unix: {}\nmode: rule\nlog-level: silent\ndns:\n  enable: false\nproxies:\n- {{name: a, type: http, server: 127.0.0.1, port: {dead_port}}}\n- {{name: b, type: http, server: 127.0.0.1, port: {dead_port}}}\nproxy-groups:\n- name: group\n  type: select\n  proxies: [a, b]\nrules: ['MATCH,group']\n", context.socket.display())).unwrap();
    let mut core = CoreSupervisor::new(
        MihomoConfig {
            binary,
            working_dir: directory.0.clone(),
            controller,
            secret: "isolated-contract-secret".into(),
            max_restarts: 0,
            restart_backoff: Duration::from_millis(20),
            stop_timeout: Duration::from_secs(2),
            log_capacity: 20,
        },
        config_path,
    )
    .unwrap();
    core.set_internal_socket(context.socket.clone());
    core.start().unwrap();
    let ready = Instant::now() + Duration::from_secs(5);
    while core.health(Duration::from_millis(100)).is_err() {
        assert!(Instant::now() < ready, "{:?}", core.logs());
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut client = context.client().unwrap();
    let snapshot = client.proxy_groups().unwrap();
    let mut pending = pending();
    pending.baseline = context.baseline(&snapshot, client.mode().unwrap()).unwrap();
    let mut report = Report::default();
    checks::apply(&context, &pending, &mut report, || Ok(())).unwrap();
    assert_eq!(report.outcome.as_deref(), Some("optimization.restored"));
    assert_eq!(report.measurements.len(), 2);
    assert!(
        client
            .proxy_groups()
            .unwrap()
            .groups
            .iter()
            .any(|g| g.name == "group" && g.selected.as_deref() == Some("a"))
    );
    core.stop().unwrap();
}

#[test]
#[ignore = "requires MIHOMO_BIN pointing to the pinned sidecar"]
fn optimization_default_check_does_not_probe_or_change_fallback() {
    use crate::mihomo::{CoreSupervisor, MihomoConfig, SidecarManifest};
    let binary = std::path::PathBuf::from(std::env::var_os("MIHOMO_BIN").expect("MIHOMO_BIN"));
    SidecarManifest::load(
        &std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/mihomo/manifest.json"),
    )
    .unwrap()
    .target("aarch64-apple-darwin")
    .unwrap()
    .verify_executable(&binary)
    .unwrap();
    let directory = Scratch::new();
    let mut context = directory.context();
    context.socket = directory.0.join("core.sock");
    let controller = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let dead_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config_path = directory.0.join("test.yaml");
    std::fs::write(&config_path, format!("external-controller-unix: {}\nmode: rule\nlog-level: silent\ndns:\n  enable: false\nproxies:\n- {{name: a, type: http, server: 127.0.0.1, port: {dead_port}}}\n- {{name: b, type: http, server: 127.0.0.1, port: {dead_port}}}\nproxy-groups:\n- name: AutoFallback\n  type: fallback\n  proxies: [a, b]\n  url: https://www.gstatic.com/generate_204\n  interval: 0\n  lazy: true\nrules: ['MATCH,AutoFallback']\n", context.socket.display())).unwrap();
    let mut core = CoreSupervisor::new(
        MihomoConfig {
            binary,
            working_dir: directory.0.clone(),
            controller,
            secret: "isolated-contract-secret".into(),
            max_restarts: 0,
            restart_backoff: Duration::from_millis(20),
            stop_timeout: Duration::from_secs(2),
            log_capacity: 20,
        },
        config_path,
    )
    .unwrap();
    core.set_internal_socket(context.socket.clone());
    core.start().unwrap();
    let ready = Instant::now() + Duration::from_secs(5);
    while core.health(Duration::from_millis(100)).is_err() {
        assert!(Instant::now() < ready, "{:?}", core.logs());
        std::thread::sleep(Duration::from_millis(20));
    }

    let mut client = context.client().unwrap();
    let before = client.proxy_groups().unwrap();
    for background in [false, true] {
        let result = checks::check(
            &context,
            1,
            None,
            background,
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        assert!(result.report.measurements.is_empty());
        assert!(
            result
                .report
                .findings
                .iter()
                .any(|f| f.code == "probes_disabled" && f.severity == Severity::Unknown)
        );
        let after = client.proxy_groups().unwrap();
        assert_eq!(before.groups, after.groups);
        assert_eq!(before.proxies["a"].delay, after.proxies["a"].delay);
        assert_eq!(before.proxies["b"].delay, after.proxies["b"].delay);
    }
    assert!(
        checks::check(
            &context,
            2,
            Some("group"),
            false,
            &AtomicBool::new(false),
            |_| {}
        )
        .is_err()
    );
    assert!(
        checks::apply(&context, &pending(), &mut Report::default(), || panic!(
            "no write"
        ))
        .is_err()
    );
    // Opt-in permits measurements; no assumption about the core's initial history
    // or independent health jobs is needed for the consent boundary.
    context.active_probes = true;
    let result = checks::check(&context, 3, None, true, &AtomicBool::new(false), |_| {}).unwrap();
    assert_eq!(result.report.measurements.len(), 2);
    assert!(result.report.measurements.iter().all(|m| m.node == "a"));
    let after = client.proxy_groups().unwrap();
    assert_eq!(after.proxies["a"].delay, Some(0));
    core.stop().unwrap();
}

#[test]
fn probe_permission_and_check_are_started_by_one_confirmed_command() {
    let directory = Scratch::new();
    let mut service = Service::new(directory.0.clone());
    assert!(!service.snapshot().active_probes);
    service
        .handle(Command::AllowProbesAndCheck, directory.context())
        .unwrap();
    assert!(service.snapshot().active_probes);
    let deadline = Instant::now() + Duration::from_secs(5);
    while service.snapshot().busy() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(service.snapshot().report.is_some());
    assert!(Service::new(directory.0.clone()).snapshot().active_probes);
}
