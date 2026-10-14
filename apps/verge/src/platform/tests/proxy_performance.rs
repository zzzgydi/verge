use super::*;

// Stateful OS boundary: command success need not imply that the OS applied it.
struct SimulatedNetwork {
    services: Vec<SystemProxyServiceState>,
    calls: Vec<Vec<String>>,
    ignore_writes: bool,
    fail_write: Option<usize>,
    writes: usize,
}

impl sysproxy::macos::CommandRunner for SimulatedNetwork {
    fn run(&mut self, args: &[&str]) -> io::Result<std::process::Output> {
        use std::os::unix::process::ExitStatusExt as _;
        self.calls
            .push(args.iter().map(|s| s.to_string()).collect());
        let command = args[0];
        let write = command.starts_with("-set");
        if write {
            self.writes += 1;
            if self.fail_write == Some(self.writes) {
                return Ok(std::process::Output {
                    status: std::process::ExitStatus::from_raw(256),
                    stdout: Vec::new(),
                    stderr: b"injected write failure".to_vec(),
                });
            }
        }
        let service = self
            .services
            .iter_mut()
            .find(|s| s.service == args[1])
            .unwrap();
        let stdout = if write && self.ignore_writes {
            String::new()
        } else if command.contains("autoproxy") {
            match command {
                "-getautoproxyurl" => format!(
                    "URL: {}\nEnabled: {}\n",
                    service.auto_proxy.url.as_deref().unwrap_or("(null)"),
                    if service.auto_proxy.enabled {
                        "Yes"
                    } else {
                        "No"
                    }
                ),
                "-setautoproxyurl" => {
                    service.auto_proxy.url = (args[2] != "\"\"").then(|| args[2].to_string());
                    service.auto_proxy.enabled = true;
                    String::new()
                }
                "-setautoproxystate" => {
                    service.auto_proxy.enabled = args[2] == "on";
                    String::new()
                }
                _ => panic!("unexpected command {command}"),
            }
        } else if command.contains("bypass") {
            if write {
                service.bypass = args[2..]
                    .iter()
                    .filter(|s| **s != "Empty")
                    .map(|s| s.to_string())
                    .collect();
                String::new()
            } else if service.bypass.is_empty() {
                format!(
                    "There aren't any bypass domains set on {}.\n",
                    service.service
                )
            } else {
                service.bypass.join("\n")
            }
        } else {
            let protocol = if command.contains("secureweb") {
                &mut service.secure_web
            } else if command.contains("socks") {
                &mut service.socks
            } else {
                &mut service.web
            };
            if !write {
                output(
                    protocol.enabled,
                    &protocol.endpoint.host,
                    protocol.endpoint.port,
                )
            } else {
                if command.ends_with("state") {
                    protocol.enabled = args[2] == "on";
                } else {
                    protocol.endpoint =
                        ProxyEndpoint::new(args[2], args[3].parse().unwrap()).unwrap();
                    protocol.enabled = true;
                }
                String::new()
            }
        };
        Ok(std::process::Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: stdout.into_bytes(),
            stderr: Vec::new(),
        })
    }
}

fn fixture(
    port: u16,
    enabled: bool,
) -> (
    TestDir,
    MacSystemProxy<SimulatedNetwork>,
    Vec<String>,
    SystemProxyTarget,
) {
    let dir = TestDir::new();
    let services: Vec<String> = (0..4).map(|i| format!("Service {i}")).collect();
    let state = services
        .iter()
        .map(|service| {
            let protocol = ProxyProtocolState {
                enabled,
                endpoint: ProxyEndpoint::new("127.0.0.1", port).unwrap(),
            };
            SystemProxyServiceState {
                service: service.clone(),
                web: protocol.clone(),
                secure_web: protocol.clone(),
                socks: protocol,
                auto_proxy: AutoProxyState {
                    enabled: false,
                    url: None,
                },
                bypass: vec!["localhost".into()],
            }
        })
        .collect();
    let proxy = MacSystemProxy::new(
        SimulatedNetwork {
            services: state,
            calls: Vec::new(),
            ignore_writes: false,
            fail_write: None,
            writes: 0,
        },
        dir.0.join("recovery.json"),
    );
    let target = SystemProxyTarget::Manual {
        endpoint: ProxyEndpoint::new("127.0.0.1", 7897).unwrap(),
        bypass: vec!["localhost".into()],
    };
    (dir, proxy, services, target)
}

#[test]
fn four_service_enable_reuses_transaction_snapshot_and_only_writes_switches() {
    let (_dir, mut proxy, services, target) = fixture(7897, false);
    let original = proxy.runner.services.clone();
    let (previous, actual) = proxy.configure_with_snapshot(&services, &target).unwrap();
    assert_eq!(previous.services, original);
    assert!(target.matches(&actual));
    assert_eq!(
        read_recovery(&proxy.recovery_path).unwrap().services,
        original
    );
    assert_eq!(
        proxy.runner.calls.len(),
        52,
        "40 reads + 12 switch writes; formerly 92 calls"
    );
    assert_eq!(proxy.runner.writes, 12);
    assert!(
        proxy
            .runner
            .calls
            .iter()
            .filter(|c| c[0].starts_with("-set"))
            .all(|c| c[0].ends_with("state"))
    );

    let original_record = fs::read(&proxy.recovery_path).unwrap();
    proxy.runner.calls.clear();
    proxy.runner.writes = 0;
    let (_, actual) = proxy.configure_with_snapshot(&services, &target).unwrap();
    assert!(target.matches(&actual));
    assert_eq!(
        proxy.runner.calls.len(),
        40,
        "idempotent enable still verifies OS state"
    );
    assert_eq!(proxy.runner.writes, 0);
    assert_eq!(fs::read(&proxy.recovery_path).unwrap(), original_record);
    assert_eq!(proxy.disable(&services).unwrap().services, original);
    assert!(!proxy.recovery_path.exists());
}

#[test]
fn changed_endpoints_are_written_and_pac_is_disabled_first() {
    let (_dir, mut proxy, services, target) = fixture(8080, false);
    for service in &mut proxy.runner.services {
        service.auto_proxy.enabled = true;
        service.auto_proxy.url = Some("http://localhost/old.pac".into());
    }
    let actual = proxy.configure(&services, &target).unwrap();
    assert!(target.matches(&actual));
    assert_eq!(proxy.runner.calls.len(), 68);
    for service in services {
        let writes: Vec<_> = proxy
            .runner
            .calls
            .iter()
            .filter(|c| c[0].starts_with("-set") && c[1] == service)
            .collect();
        assert_eq!(writes[0][0], "-setautoproxystate");
        assert_eq!(writes[0][2], "off");
    }
}

#[test]
fn cached_pac_url_only_needs_its_switch_and_repeated_enable_has_no_writes() {
    let (_dir, mut proxy, services, _) = fixture(7897, false);
    let url = "http://localhost/proxy.pac";
    for service in &mut proxy.runner.services {
        service.auto_proxy.url = Some(url.into());
    }
    let target = SystemProxyTarget::Pac { url: url.into() };
    assert!(target.matches(&proxy.configure(&services, &target).unwrap()));
    assert_eq!(proxy.runner.calls.len(), 44);
    assert_eq!(proxy.runner.writes, 4);
    proxy.runner.calls.clear();
    proxy.runner.writes = 0;
    assert!(target.matches(&proxy.configure(&services, &target).unwrap()));
    assert_eq!(proxy.runner.calls.len(), 40);
    assert_eq!(proxy.runner.writes, 0);
}

#[test]
fn ignored_switch_write_is_detected_and_original_state_is_restored() {
    let (_dir, mut proxy, services, target) = fixture(7897, false);
    let original = proxy.runner.services.clone();
    proxy.runner.ignore_writes = true;
    assert!(
        proxy
            .configure(&services, &target)
            .unwrap_err()
            .message
            .contains("did not match")
    );
    assert_eq!(proxy.runner.services, original);
    assert!(!proxy.recovery_path.exists());
}

#[test]
fn every_delta_write_failure_rolls_back_all_services() {
    for fail in 1..=12 {
        let (_dir, mut proxy, services, target) = fixture(7897, false);
        let original = proxy.runner.services.clone();
        proxy.runner.fail_write = Some(fail);
        assert!(proxy.configure(&services, &target).is_err());
        assert_eq!(proxy.runner.services, original);
        assert!(!proxy.recovery_path.exists());
    }
}

#[test]
fn settings_save_rollback_uses_current_snapshot_not_original_recovery() {
    let (_dir, mut proxy, services, target) = fixture(7897, false);
    proxy.configure(&services, &target).unwrap();
    let original_record = fs::read(&proxy.recovery_path).unwrap();
    let current = proxy.runner.services.clone();
    let next = SystemProxyTarget::Pac {
        url: "http://localhost/next.pac".into(),
    };
    let (previous, actual) = proxy.configure_with_snapshot(&services, &next).unwrap();
    assert_eq!(previous.services, current);
    assert!(next.matches(&actual));
    proxy.restore_snapshot(&previous).unwrap();
    assert_eq!(proxy.runner.services, current);
    assert_eq!(fs::read(&proxy.recovery_path).unwrap(), original_record);
    assert!(target.matches(&proxy.state(&services).unwrap()));
}

#[test]
fn external_disable_verifies_switches_without_a_second_post_read() {
    let (_dir, mut proxy, services, _) = fixture(7897, true);
    let actual = proxy.disable(&services).unwrap();
    assert!(!actual.unified_enabled());
    assert_eq!(proxy.runner.calls.len(), 52);
    assert!(!proxy.recovery_path.exists());

    let (_dir, mut proxy, services, _) = fixture(7897, true);
    proxy.runner.ignore_writes = true;
    assert!(
        proxy
            .disable(&services)
            .unwrap_err()
            .message
            .contains("remained enabled")
    );
    assert!(proxy.runner.services.iter().all(|s| s.web.enabled));
}
