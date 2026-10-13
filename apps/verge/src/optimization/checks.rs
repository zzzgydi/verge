use super::*;
use crate::{
    config::FileProfileStore,
    domain::{ProxyGroup, ProxySnapshot, RunMode, SystemProxyState, SystemProxyTarget},
    mihomo::{ControllerTransport, MihomoClient, TcpControllerTransport},
    platform::{MacSystemProxy, SystemProxyRunner},
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(crate) struct Context {
    pub socket: PathBuf,
    pub profiles: FileProfileStore,
    pub services: Vec<String>,
    pub recovery: PathBuf,
    pub target: Option<SystemProxyTarget>,
    pub tun_connected: bool,
    pub errors: Option<usize>,
    pub connections: Option<usize>,
    pub epoch: Arc<AtomicU64>,
    pub active_probes: bool,
    pub sample_cursor: u64,
}

impl Context {
    pub fn client(&self) -> Result<MihomoClient<TcpControllerTransport>, AppError> {
        Ok(MihomoClient::new(TcpControllerTransport::unix(
            self.socket.clone(),
            Duration::from_secs(2),
        )?))
    }
    pub fn baseline(&self, snapshot: &ProxySnapshot, mode: RunMode) -> Result<String, AppError> {
        use std::os::unix::fs::MetadataExt as _;
        let socket = std::fs::metadata(&self.socket).map_err(|_| error("optimization.stale"))?;
        let network = self.client()?.network_settings()?;
        Ok(crate::ai::proposals::digest(format!(
            "{}:{:?}:{mode:?}:{}:{}:{network:?}:{}:{}",
            self.profiles.preview_digest()?,
            snapshot.groups,
            fingerprint()?,
            self.epoch.load(Ordering::Relaxed),
            socket.ino(),
            socket.ctime()
        )))
    }
}

/// Hash active interface addresses locally. No addresses or interface names leave this function.
pub(crate) fn fingerprint() -> Result<String, AppError> {
    use std::{
        ffi::CStr,
        net::{Ipv4Addr, Ipv6Addr},
    };
    let mut first = std::ptr::null_mut();
    // SAFETY: getifaddrs owns this linked list until freeifaddrs; sockaddr family guards casts.
    unsafe {
        if libc::getifaddrs(&mut first) != 0 {
            return Err(error("optimization.network_unknown"));
        }
        let mut entries = Vec::new();
        let mut item = first;
        while !item.is_null() {
            let interface = &*item;
            if !interface.ifa_addr.is_null() && interface.ifa_flags & libc::IFF_UP as u32 != 0 {
                let name = CStr::from_ptr(interface.ifa_name).to_string_lossy();
                let address = match (*interface.ifa_addr).sa_family as i32 {
                    libc::AF_INET => Some(
                        Ipv4Addr::from(
                            (*(interface.ifa_addr as *const libc::sockaddr_in))
                                .sin_addr
                                .s_addr
                                .to_ne_bytes(),
                        )
                        .to_string(),
                    ),
                    libc::AF_INET6 => Some(
                        Ipv6Addr::from(
                            (*(interface.ifa_addr as *const libc::sockaddr_in6))
                                .sin6_addr
                                .s6_addr,
                        )
                        .to_string(),
                    ),
                    _ => None,
                };
                if let Some(address) = address {
                    entries.push(format!("{name}:{address}"));
                }
            }
            item = interface.ifa_next;
        }
        libc::freeifaddrs(first);
        if entries.is_empty() {
            return Err(error("optimization.network_unknown"));
        }
        entries.sort();
        Ok(crate::ai::proposals::digest(entries.join("\n")))
    }
}

pub(super) fn leaf(snapshot: &ProxySnapshot, name: &str) -> Option<String> {
    let mut current = name;
    let mut visited = std::collections::HashSet::new();
    loop {
        if !visited.insert(current) || visited.len() > 16 {
            return None;
        }
        if let Some(group) = snapshot.groups.iter().find(|g| g.name == current) {
            current = group.selected.as_deref()?;
        } else {
            return eligible(snapshot, current).then(|| current.to_owned());
        }
    }
}

pub(super) fn eligible(snapshot: &ProxySnapshot, name: &str) -> bool {
    !snapshot.groups.iter().any(|g| g.name == name)
        && snapshot.proxies.get(name).is_some_and(|p| {
            !["Direct", "Reject", "RejectDrop", "Pass", "Compatible"]
                .iter()
                .any(|s| p.kind.eq_ignore_ascii_case(s))
        })
        && !["DIRECT", "REJECT", "REJECT-DROP", "PASS", "COMPATIBLE"].contains(&name)
        && crate::ai::tools::label(name) == name
        && !name.is_empty()
}

pub(super) fn selected_group<'a>(
    snapshot: &'a ProxySnapshot,
    name: &str,
) -> Option<&'a ProxyGroup> {
    snapshot.groups.iter().find(|g| {
        g.name == name
            && g.kind.eq_ignore_ascii_case("selector")
            && crate::ai::tools::label(&g.name) == g.name
            && g.selected
                .as_deref()
                .is_some_and(|n| g.members.iter().any(|m| m == n) && eligible(snapshot, n))
    })
}

pub(super) struct Pending {
    pub plan: Plan,
    pub baseline: String,
    pub created: Instant,
}

pub(super) struct CheckResult {
    pub report: Report,
    pub pending: Option<Pending>,
}

fn finding(report: &mut Report, code: &str, severity: Severity, detail: impl Into<String>) {
    report.findings.push(Finding {
        code: code.into(),
        severity,
        detail: detail.into(),
    });
}

pub(super) fn measure<T: ControllerTransport>(
    client: &mut MihomoClient<T>,
    node: &str,
    report: &mut Report,
    cancel: &AtomicBool,
    deadline: Instant,
) {
    for (target, url) in PROBES.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) || Instant::now() + Duration::from_secs(5) > deadline {
            report.partial = true;
            return;
        }
        report.measurements.push(Measurement {
            node: node.into(),
            target,
            delay_ms: client.delay(node, url, 2000).ok().filter(|d| *d > 0),
            captured_at: now(),
        });
    }
}

pub(super) fn healthy(report: &Report, node: &str) -> bool {
    (0..PROBES.len()).all(|target| {
        report
            .measurements
            .iter()
            .any(|m| m.node == node && m.target == target && m.delay_ms.is_some_and(|ms| ms > 0))
    })
}

pub(super) fn failed_twice(report: &Report, node: &str) -> bool {
    (0..PROBES.len()).all(|target| {
        report
            .measurements
            .iter()
            .filter(|m| m.node == node && m.target == target && m.delay_ms.is_none())
            .count()
            >= 2
    })
}

pub(crate) fn proxy_severity(
    platform: &Result<SystemProxyState, AppError>,
    target: Option<&SystemProxyTarget>,
) -> Severity {
    match (platform, target) {
        // An owned proxy keeps the original settings on disk until it is disabled.
        (Ok(state), Some(target)) if target.matches(state) => Severity::Pass,
        (Ok(_), Some(_)) => Severity::Warning,
        (Ok(state), None) if state.recovery_pending => Severity::Warning,
        (Ok(_), None) => Severity::Notice,
        _ => Severity::Unknown,
    }
}

pub(super) fn sampled_nodes(nodes: &[String], cursor: u64, limit: usize) -> Vec<&String> {
    if nodes.is_empty() {
        return Vec::new();
    }
    let start = (cursor % nodes.len() as u64) as usize;
    nodes
        .iter()
        .cycle()
        .skip(start)
        .take(limit.min(nodes.len()))
        .collect()
}

pub(super) fn check(
    context: &Context,
    id: u64,
    group: Option<&str>,
    background: bool,
    cancel: &AtomicBool,
    publish: impl Fn(&Report),
) -> Result<CheckResult, AppError> {
    if group.is_some() && !context.active_probes {
        return Err(error("optimization.probes_required"));
    }
    let start = Instant::now();
    let deadline = start + Duration::from_secs(if group.is_some() { 45 } else { 30 });
    let mut report = Report {
        id,
        captured_at: now(),
        background,
        ..Default::default()
    };
    let mut pending = None;
    finding(
        &mut report,
        "profile",
        if context.profiles.selected().is_some() {
            Severity::Pass
        } else {
            Severity::Unknown
        },
        "",
    );
    publish(&report);
    let mut client = context.client()?;
    if client.version().is_err() {
        finding(&mut report, "core", Severity::Warning, "");
        report.partial = true;
    } else {
        finding(&mut report, "core", Severity::Pass, "");
        let mode = client.mode()?;
        if mode == RunMode::Direct && group.is_some() {
            return Err(error("optimization.group_unavailable"));
        }
        let proxies = client.proxy_groups()?;
        let baseline = context.baseline(&proxies, mode)?;
        report.groups = proxies
            .groups
            .iter()
            .filter(|g| selected_group(&proxies, &g.name).is_some())
            .take(32)
            .map(|g| g.name.clone())
            .collect();
        let platform = MacSystemProxy::new(SystemProxyRunner, context.recovery.clone())
            .state(&context.services);
        let proxy_severity = proxy_severity(&platform, context.target.as_ref());
        finding(&mut report, "system_proxy", proxy_severity, "");
        match client.network_settings() {
            Ok(network) => finding(
                &mut report,
                "tun",
                if network.tun_enabled && !context.tun_connected {
                    Severity::Warning
                } else {
                    Severity::Notice
                },
                "",
            ),
            Err(_) => finding(&mut report, "tun", Severity::Unknown, ""),
        }
        let dns = context
            .profiles
            .runtime_bytes()
            .ok()
            .and_then(|bytes| serde_yaml::from_slice::<serde_yaml::Value>(&bytes).ok())
            .and_then(|yaml| yaml["dns"]["enable"].as_bool());
        finding(
            &mut report,
            "dns",
            if dns == Some(true) {
                Severity::Notice
            } else {
                Severity::Unknown
            },
            "",
        );
        finding(
            &mut report,
            "logs",
            match context.errors {
                Some(0) => Severity::Notice,
                Some(_) => Severity::Warning,
                None => Severity::Unknown,
            },
            context.errors.map(|n| n.to_string()).unwrap_or_default(),
        );
        finding(
            &mut report,
            "connections",
            if context.connections.is_some() {
                Severity::Notice
            } else {
                Severity::Unknown
            },
            context
                .connections
                .map(|n| n.to_string())
                .unwrap_or_default(),
        );
        publish(&report);
        if !context.active_probes {
            finding(&mut report, "probes_disabled", Severity::Unknown, "");
        } else if let Some(group_name) = group {
            let selected = selected_group(&proxies, group_name)
                .ok_or_else(|| error("optimization.group_unavailable"))?;
            let previous = selected.selected.as_ref().unwrap();
            measure(&mut client, previous, &mut report, cancel, deadline);
            publish(&report);
            if healthy(&report, previous) {
                report.outcome = Some("optimization.no_change".into());
            } else if !report.partial && !cancel.load(Ordering::Relaxed) {
                measure(&mut client, previous, &mut report, cancel, deadline);
                publish(&report);
                if !failed_twice(&report, previous) {
                    report.outcome = Some("optimization.inconclusive".into());
                } else {
                    for candidate in selected
                        .members
                        .iter()
                        .filter(|n| *n != previous && eligible(&proxies, n))
                        .take(3)
                    {
                        measure(&mut client, candidate, &mut report, cancel, deadline);
                        publish(&report);
                        if healthy(&report, candidate) {
                            let mut random = [0u8; 16];
                            getrandom::fill(&mut random)
                                .map_err(|_| error("optimization.failed"))?;
                            let plan = Plan {
                                id: hex::encode(random),
                                group: group_name.into(),
                                previous: previous.clone(),
                                proposed: candidate.clone(),
                                expires_at: now() + 120,
                            };
                            report.plan = Some(plan.clone());
                            pending = Some(Pending {
                                plan,
                                baseline: baseline.clone(),
                                created: Instant::now(),
                            });
                            break;
                        }
                        if report.partial || cancel.load(Ordering::Relaxed) {
                            break;
                        }
                    }
                    if pending.is_none() {
                        report.outcome = Some("optimization.no_candidate".into());
                    }
                }
            }
        } else if mode == RunMode::Direct {
            finding(&mut report, "direct_mode", Severity::Notice, "");
        } else {
            let mut nodes = Vec::new();
            for g in &proxies.groups {
                if mode == RunMode::Global && g.name != "GLOBAL" {
                    continue;
                }
                if mode == RunMode::Rule && g.name == "GLOBAL" {
                    continue;
                }
                if let Some(node) = leaf(&proxies, &g.name)
                    && !nodes.contains(&node)
                {
                    nodes.push(node);
                }
            }
            if nodes.is_empty() {
                finding(&mut report, "routes", Severity::Unknown, "");
            }
            if nodes.len() > 3 {
                report.partial = true;
            }
            for node in sampled_nodes(
                &nodes,
                context.sample_cursor,
                if background { 1 } else { 3 },
            ) {
                measure(&mut client, node, &mut report, cancel, deadline);
                publish(&report);
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
            }
            if background && nodes.len() > 1 {
                report.partial = true;
            }
        }
        let latest = client.proxy_groups()?;
        if context.baseline(&latest, client.mode()?)? != baseline {
            report.partial = true;
            report.plan = None;
            pending = None;
            finding(&mut report, "stale", Severity::Unknown, "");
        }
    }
    report.cancelled = cancel.load(Ordering::Relaxed);
    if report.cancelled {
        report.plan = None;
        pending = None;
    }
    report.completed_at = now();
    Ok(CheckResult { report, pending })
}

pub(super) fn apply(
    context: &Context,
    pending: &Pending,
    report: &mut Report,
    intent: impl FnOnce() -> Result<(), AppError>,
) -> Result<(), AppError> {
    if !context.active_probes {
        return Err(error("optimization.probes_required"));
    }
    apply_verified(
        &mut context.client()?,
        pending,
        report,
        |snapshot, mode| context.baseline(snapshot, mode),
        intent,
    )
}

pub(super) fn apply_verified<T: ControllerTransport>(
    client: &mut MihomoClient<T>,
    pending: &Pending,
    report: &mut Report,
    baseline: impl Fn(&ProxySnapshot, RunMode) -> Result<String, AppError>,
    intent: impl FnOnce() -> Result<(), AppError>,
) -> Result<(), AppError> {
    let plan = &pending.plan;
    if pending.created.elapsed() > Duration::from_secs(120) {
        return Err(error("optimization.expired"));
    }
    let proxies = client.proxy_groups()?;
    let mode = client.mode()?;
    if baseline(&proxies, mode)? != pending.baseline {
        return Err(error("optimization.stale"));
    }
    let group = selected_group(&proxies, &plan.group).ok_or_else(|| error("optimization.stale"))?;
    if group.selected.as_ref() != Some(&plan.previous)
        || !group.members.contains(&plan.proposed)
        || !eligible(&proxies, &plan.proposed)
    {
        return Err(error("optimization.stale"));
    }
    let mut expected = proxies.clone();
    expected
        .groups
        .iter_mut()
        .find(|g| g.name == plan.group)
        .unwrap()
        .selected = Some(plan.proposed.clone());
    let expected_baseline = baseline(&expected, mode)?;
    if baseline(&proxies, mode)? != pending.baseline {
        return Err(error("optimization.stale"));
    }
    // Persist an intent before the first write. A crash must never look like a successful check.
    intent()?;
    // A failed write can still have reached the core. Always read it back before deciding recovery.
    let _write = client.select_proxy(&plan.group, &plan.proposed);
    let after = client.proxy_groups()?;
    let selected = after
        .groups
        .iter()
        .find(|g| g.name == plan.group)
        .and_then(|g| g.selected.as_ref());
    if selected == Some(&plan.previous) {
        report.outcome = Some("optimization.not_applied".into());
        return Ok(());
    }
    if selected != Some(&plan.proposed) {
        return Err(error("optimization.apply_unknown"));
    }
    if baseline(&after, client.mode()?)? != expected_baseline {
        return Err(error("optimization.apply_unknown"));
    }
    let mut verification = Report::default();
    measure(
        client,
        &plan.proposed,
        &mut verification,
        &AtomicBool::new(false),
        Instant::now() + Duration::from_secs(12),
    );
    let success = healthy(&verification, &plan.proposed);
    report.measurements.extend(verification.measurements);
    let current = client.proxy_groups()?;
    if baseline(&current, client.mode()?)? != expected_baseline {
        return Err(error("optimization.apply_unknown"));
    }
    if success {
        report.outcome = Some("optimization.applied".into());
        return Ok(());
    }
    let _restore = client.select_proxy(&plan.group, &plan.previous);
    if client
        .proxy_groups()?
        .groups
        .iter()
        .any(|g| g.name == plan.group && g.selected.as_ref() == Some(&plan.previous))
    {
        report.outcome = Some("optimization.restored".into());
        Ok(())
    } else {
        Err(error("optimization.recovery_failed"))
    }
}
