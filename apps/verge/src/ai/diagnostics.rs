//! Bounded on-demand diagnostics and local proposals. Runs only on AI workers.
mod dns;
mod network;
mod proxy;
use super::{
    error,
    proposals::{self, Action, Change, Proposals},
    tools::{self, Evidence, ToolContext},
};
use crate::{
    config::{ConfigPreview, FileProfileStore, MergeConfig},
    domain::{AppError, RunMode},
    mihomo::{MihomoClient, TcpControllerTransport},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub(super) const NAMES: [&str; 15] = [
    "test_nodes",
    "explain_rules",
    "suggest_node",
    "suggest_mode",
    "preview_merge",
    "query_dns",
    "dns_status",
    "inspect_connections",
    "read_logs",
    "tun_status",
    "suggest_system_proxy",
    "suggest_tun",
    "preview_dns",
    "system_proxy_settings",
    "preview_system_proxy_settings",
];

#[derive(Clone)]
pub struct ConfigContext {
    pub profiles: FileProfileStore,
    pub binary: PathBuf,
    pub working_dir: PathBuf,
    pub controller: SocketAddr,
    pub secret: String,
}

pub(crate) struct Session {
    pub context: ToolContext,
    pub evidence: Vec<Evidence>,
    pub run_id: u64,
    pub proposals: Arc<Mutex<Proposals>>,
    pub views: Vec<super::proposals::Proposal>,
    pub cancel: Arc<AtomicBool>,
    tested: HashSet<String>,
}

impl Session {
    pub fn new(
        context: ToolContext,
        evidence: Vec<Evidence>,
        run_id: u64,
        proposals: Arc<Mutex<Proposals>>,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        Self {
            context,
            evidence,
            run_id,
            proposals,
            cancel,
            views: Vec::new(),
            tested: HashSet::new(),
        }
    }
    fn client(&self) -> Result<MihomoClient<TcpControllerTransport>, AppError> {
        Ok(MihomoClient::new(TcpControllerTransport::unix(
            self.context.socket.clone(),
            Duration::from_secs(2),
        )?))
    }
    fn record(&mut self, source: &str, data: Value) -> Result<Evidence, AppError> {
        if self.evidence.len() >= 24 || serde_json::to_vec(&data).unwrap().len() > 24 * 1024 {
            return Err(error("Diagnostic evidence limit reached"));
        }
        let evidence = Evidence {
            id: format!("R{}-E{}", self.run_id, self.evidence.len() + 1),
            source: source.into(),
            captured_at: proposals::now(),
            data,
        };
        self.evidence.push(evidence.clone());
        Ok(evidence)
    }
    fn config(&self) -> Result<&ConfigContext, AppError> {
        self.context
            .config
            .as_ref()
            .ok_or_else(|| error("Configuration preview unavailable"))
    }
    fn snapshot(&self) -> Result<ConfigPreview, AppError> {
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| error("Unable to create preview"))?;
        let name = nonce.iter().map(|b| format!("{b:02x}")).collect::<String>();
        self.config()?
            .profiles
            .isolated_preview(std::env::temp_dir().join(format!("verge-ai-{name}")))
    }
    fn propose(
        &mut self,
        action: Action,
        baseline: String,
        kind: &str,
        target: String,
        changes: Vec<Change>,
        evidence: &Evidence,
    ) -> Result<Value, AppError> {
        let mut proposals = self.proposals.lock().unwrap();
        self.check_cancel()?;
        let proposal = proposals.insert(
            self.run_id,
            action,
            baseline,
            kind,
            target,
            changes,
            evidence,
        )?;
        // Confirmation digest and raw parameters stay local; models only see the opaque proposal ID.
        let result = json!({"proposal_id":proposal.id,"status":"awaiting_user_confirmation","evidence":evidence});
        self.views.push(proposal);
        Ok(result)
    }
    fn check_cancel(&self) -> Result<(), AppError> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(error("Cancelled"))
        } else {
            Ok(())
        }
    }
    pub fn execute(&mut self, name: &str, args: &str) -> Result<Value, AppError> {
        self.check_cancel()?;
        if tools::NAMES.contains(&name) && name != "config_check" {
            return tools::execute(name, args, &self.evidence).map(|e| json!(e));
        }
        if ["system_proxy_settings", "preview_system_proxy_settings"].contains(&name) {
            return self.proxy_settings_tool(name, args);
        }
        if network::NAMES.contains(&name) {
            return self.network_tool(name, args);
        }
        match name {
            "config_check" => {
                let _: Empty = parse(args)?;
                let preview = self.snapshot()?;
                let selected = preview
                    .store
                    .selected()
                    .cloned()
                    .ok_or_else(|| error("Select a profile first"))?;
                let config = self.config()?;
                let path = preview.store.materialize_runtime(
                    &selected,
                    config.controller,
                    &config.secret,
                )?;
                validate(&config.binary, &path, &config.working_dir, &self.cancel)?;
                let e = self.record(
                    name,
                    json!({"selected":true,"mihomo_validated":true,"config_content_omitted":true}),
                )?;
                Ok(json!(e))
            }
            "test_nodes" => {
                let request: Nodes = parse(args)?;
                if request.nodes.is_empty() || request.nodes.len() > 6 {
                    return Err(error("Select one to six nodes"));
                }
                let mut client = self.client()?;
                let snapshot = client.proxy_groups()?;
                let group = snapshot
                    .groups
                    .iter()
                    .find(|g| g.name == request.group)
                    .ok_or_else(|| error("Unknown proxy group"))?;
                let mut results = Vec::new();
                if request.nodes.iter().any(|node| {
                    !group.members.contains(node)
                        || tools::label(node) != *node
                        || ["REJECT", "REJECT-DROP"].contains(&node.as_str())
                }) {
                    return Err(error("Node is not an available member of that group"));
                }
                for node in request.nodes {
                    self.check_cancel()?;
                    if self.tested.contains(&node) {
                        continue;
                    }
                    if self.tested.len() >= 6 {
                        return Err(error("Node test budget exceeded"));
                    }
                    self.tested.insert(node.clone());
                    let result = client.delay(&node, "https://www.gstatic.com/generate_204", 2500);
                    results.push(match result {
                        Ok(ms) => json!({"node":node,"delay_ms":ms,"ok":true}),
                        Err(e) => json!({"node":node,"ok":false,"error_code":e.code}),
                    });
                }
                results.sort_by_key(|v| v["delay_ms"].as_u64().unwrap_or(u64::MAX));
                let e=self.record(name,json!({"group":tools::label(&request.group),"results":results,
                    "probe":"https://www.gstatic.com/generate_204","scope":"single HTTP latency probe; not a bandwidth or all-sites test"}))?;
                Ok(json!(e))
            }
            "explain_rules" => {
                let request: Host = parse(args)?;
                let host = validated_host(&request.host)?;
                let mut matches = Vec::new();
                let mut unsupported = 0;
                let rules = self.client()?.rules()?;
                for (i, rule) in rules.iter().enumerate() {
                    let payload = rule.payload.to_ascii_lowercase();
                    let matched = match rule.kind.as_str() {
                        "Domain" => host == payload,
                        "DomainSuffix" => host == payload || host.ends_with(&format!(".{payload}")),
                        "DomainKeyword" => host.contains(&payload),
                        "Match" => true,
                        _ => {
                            unsupported += 1;
                            false
                        }
                    };
                    if matched && matches.len() < 12 {
                        matches.push(json!({"position":i+1,"kind":rule.kind,"outbound":tools::label(&rule.proxy)}));
                    }
                }
                let e=self.record(name,json!({"host":host,"matching_rules":matches,"unsupported_rules":unsupported,
                    "observed_connections":self.routing_evidence(&host),"rule_providers":self.ruleset_evidence(),
                    "scope":"static domain matches are candidates only; unsupported rules are not evaluated. Observed connections show actual routing at connection creation, including rule sets; not a prediction for new traffic"}))?;
                Ok(json!(e))
            }
            "suggest_mode" => {
                let request: Mode = parse(args)?;
                let previous = self.client()?.mode()?;
                if previous == request.mode {
                    return Err(error("This mode is already active"));
                }
                let baseline = self.config()?.profiles.preview_digest()?;
                let e = self.record(
                    name,
                    json!({"previous":previous,"proposed":request.mode,"applied":false}),
                )?;
                self.propose(
                    Action::Mode {
                        mode: request.mode,
                        previous,
                    },
                    baseline,
                    "mode",
                    format!("{:?}", request.mode),
                    vec![Change {
                        path: "mode".into(),
                        before: format!("{previous:?}"),
                        after: format!("{:?}", request.mode),
                    }],
                    &e,
                )
            }
            "suggest_node" => {
                let request: Node = parse(args)?;
                let snapshot = self.client()?.proxy_groups()?;
                let group = snapshot
                    .groups
                    .iter()
                    .find(|g| g.name == request.group && g.kind.eq_ignore_ascii_case("selector"))
                    .ok_or_else(|| error("Choose a manually selectable proxy group"))?;
                if !group.members.contains(&request.node)
                    || tools::label(&request.node) != request.node
                    || tools::label(&request.group) != request.group
                {
                    return Err(error("Unknown or unsupported node"));
                }
                let previous = group
                    .selected
                    .clone()
                    .ok_or_else(|| error("Current node unavailable"))?;
                if previous == request.node {
                    return Err(error("This node is already selected"));
                }
                let baseline = self.config()?.profiles.preview_digest()?;
                let measurements = self
                    .evidence
                    .iter()
                    .filter(|e| e.source == "test_nodes")
                    .filter_map(|e| e.data["results"].as_array().map(|r| (e, r)))
                    .flat_map(|(e, rows)| {
                        rows.iter().filter(|r| r["node"] == request.node).map(
                            |r| json!({"evidence_id":e.id,"captured_at":e.captured_at,"result":r}),
                        )
                    })
                    .collect::<Vec<_>>();
                let e=self.record(name,json!({"group":request.group,"previous":tools::label(&previous),"proposed":request.node,"measurements":measurements,"applied":false}))?;
                let changes = vec![Change {
                    path: request.group.clone(),
                    before: tools::label(&previous),
                    after: request.node.clone(),
                }];
                self.propose(
                    Action::Select {
                        group: request.group.clone(),
                        proxy: request.node,
                        previous,
                    },
                    baseline,
                    "node",
                    request.group,
                    changes,
                    &e,
                )
            }
            "preview_merge" => {
                let raw: Value = parse(args)?;
                if raw
                    .get("merge")
                    .and_then(|v| v.as_object())
                    .is_none_or(|m| m.len() != 1 || !m.contains_key("rules"))
                    || raw["merge"]["rules"].as_array().is_some_and(|ops| {
                        ops.iter().any(|op| {
                            op.as_object().is_none_or(|m| {
                                m.keys()
                                    .any(|k| !["key", "op", "value", "items"].contains(&k.as_str()))
                            })
                        })
                    })
                {
                    return Err(error("Unknown Merge fields"));
                }
                let request: Merge = parse(args)?;
                validate_merge(&request.merge)?;
                let mut client = self.client()?;
                let runtime = proposals::RuntimeBaseline::new(
                    client.mode()?,
                    client.network_settings()?,
                    client.proxy_groups()?,
                );
                let previous_runtime = proposals::digest(self.config()?.profiles.runtime_bytes()?);
                let mut preview = self.snapshot()?;
                let selected = preview
                    .store
                    .selected()
                    .cloned()
                    .ok_or_else(|| error("Select a profile first"))?;
                let config = self.config()?;
                let originals = preview
                    .store
                    .list()
                    .iter()
                    .map(|p| {
                        preview
                            .store
                            .merged_yaml(&p.id)
                            .map(|yaml| (p.id.clone(), yaml))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let before = originals
                    .iter()
                    .find(|(id, _)| id == &selected)
                    .unwrap()
                    .1
                    .clone();
                let previous = preview.store.merge().clone();
                let mut merged = previous.clone();
                let operations =
                    serde_yaml::to_string(&request.merge).map_err(|_| error("Invalid Merge"))?;
                merged.rules.extend(request.merge.rules);
                merged.validate()?;
                let yaml = serde_yaml::to_string(&merged).map_err(|_| error("Invalid Merge"))?;
                preview.store.set_merge_yaml(&yaml)?;
                let mut changes = diff(&before, &preview.store.merged_yaml(&selected)?)?;
                if changes.is_empty() {
                    return Err(error(
                        "Merge has no effect after scripts and network overrides",
                    ));
                }
                changes.push(Change {
                    path: "Added Merge operations".into(),
                    before: String::new(),
                    after: operations,
                });
                if preview.store.list().len() > 1 {
                    changes.push(Change {
                        path: "scope".into(),
                        before: "global Merge".into(),
                        after: format!("{} profiles", preview.store.list().len()),
                    });
                }
                let mut selected_digest = String::new();
                let mut selected_candidate = Vec::new();
                for profile in preview.store.list() {
                    self.check_cancel()?;
                    let path = preview.store.materialize_runtime(
                        &profile.id,
                        config.controller,
                        &config.secret,
                    )?;
                    validate(&config.binary, &path, &config.working_dir, &self.cancel)?;
                    if profile.id == selected {
                        selected_candidate = crate::config::read_bounded(&path, 16 * 1024 * 1024)
                            .map_err(|_| error("Preview read failed"))?;
                        selected_digest = proposals::digest(&selected_candidate);
                    } else {
                        let original = &originals
                            .iter()
                            .find(|(id, _)| id == &profile.id)
                            .unwrap()
                            .1;
                        let candidate = preview.store.merged_yaml(&profile.id)?;
                        let additional = diff(original, &candidate)?;
                        for mut change in additional {
                            change.path =
                                format!("{} / {}", tools::label(&profile.name), change.path);
                            changes.push(change);
                        }
                    }
                }
                let baseline = preview.baseline.clone();
                let count = preview.store.list().len();
                let e=self.record(name,json!({"validated_profiles":count,"mihomo_validated":true,"changes":changes,"applied":false}))?;
                self.propose(
                    Action::Merge {
                        network: None,
                        yaml,
                        preview: Box::new(preview),
                        selected,
                        candidate: selected_candidate,
                        runtime_digest: selected_digest,
                        previous_runtime,
                        runtime,
                    },
                    baseline,
                    "merge",
                    "Global Merge".into(),
                    changes,
                    &e,
                )
            }
            _ => Err(error("Unknown diagnostic tool")),
        }
    }
}

fn parse<T: serde::de::DeserializeOwned>(args: &str) -> Result<T, AppError> {
    if args.len() > 16 * 1024 {
        return Err(error("Tool arguments exceed 16 KiB"));
    }
    serde_json::from_str(args).map_err(|_| error("Invalid tool arguments"))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Nodes {
    group: String,
    nodes: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Node {
    group: String,
    node: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Mode {
    mode: RunMode,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Host {
    host: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Merge {
    merge: MergeConfig,
}

fn validated_host(host: &str) -> Result<String, AppError> {
    if host.is_empty()
        || host.len() > 253
        || !host
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-".contains(&c))
    {
        return Err(error("Enter a domain name without a URL or path"));
    }
    if host.trim_end_matches('.').split('.').any(|part| {
        part.is_empty() || part.len() > 63 || part.starts_with('-') || part.ends_with('-')
    }) {
        return Err(error("Invalid domain name"));
    }
    Ok(host.trim_end_matches('.').to_ascii_lowercase())
}

pub(crate) fn validate_merge(merge: &MergeConfig) -> Result<(), AppError> {
    use crate::config::MergeOp;
    merge.validate()?;
    if merge.rules.is_empty() || merge.rules.len() > 8 {
        return Err(error("Suggest one to eight Merge operations"));
    }
    for rule in &merge.rules {
        match (&*rule.key, &rule.op) {
            ("rules", MergeOp::Prepend { items } | MergeOp::Append { items })
                if items.len() <= 20 =>
            {
                for item in items {
                    let s = item
                        .as_str()
                        .ok_or_else(|| error("Expected a domain rule"))?;
                    let parts = s.split(',').collect::<Vec<_>>();
                    if parts.len() != 3
                        || !["DOMAIN", "DOMAIN-SUFFIX", "DOMAIN-KEYWORD"].contains(&parts[0])
                        || validated_host(parts[1]).is_err()
                        || tools::label(parts[2]) != parts[2]
                        || parts[2].is_empty()
                    {
                        return Err(error(
                            "AI Merge supports domain rules with an explicit outbound",
                        ));
                    }
                }
            }
            ("ipv6" | "unified-delay", MergeOp::Override { value }) if value.is_bool() => {}
            ("log-level", MergeOp::Override { value })
                if value.as_str().is_some_and(|s| {
                    ["silent", "error", "warning", "info", "debug"].contains(&s)
                }) => {}
            _ => {
                return Err(error(
                    "AI Merge supports adding domain rules, IPv6, unified delay and log level only",
                ));
            }
        }
    }
    Ok(())
}

fn diff(before: &str, after: &str) -> Result<Vec<Change>, AppError> {
    let before: serde_yaml::Value =
        serde_yaml::from_str(before).map_err(|_| error("Invalid preview"))?;
    let after: serde_yaml::Value =
        serde_yaml::from_str(after).map_err(|_| error("Invalid preview"))?;
    let mut changes = Vec::new();
    for key in ["rules", "ipv6", "unified-delay", "log-level"] {
        if before.get(key) != after.get(key) {
            let render = |v: Option<&serde_yaml::Value>| {
                if key == "rules" {
                    return format!(
                        "{} rules",
                        v.and_then(|v| v.as_sequence()).map_or(0, Vec::len)
                    );
                }
                v.and_then(|v| serde_yaml::to_string(v).ok())
                    .unwrap_or_else(|| "unset".into())
                    .trim()
                    .to_owned()
            };
            if key == "rules" {
                let old = before
                    .get(key)
                    .and_then(|v| v.as_sequence())
                    .cloned()
                    .unwrap_or_default();
                let new = after
                    .get(key)
                    .and_then(|v| v.as_sequence())
                    .cloned()
                    .unwrap_or_default();
                // Preserve the exact ordering of all pre-existing rules.
                let mut remaining = Vec::new();
                let mut cursor = 0;
                let mut positions = Vec::new();
                for (index, value) in new.iter().enumerate() {
                    if old.get(cursor) == Some(value) {
                        cursor += 1;
                    } else {
                        remaining.push(value.clone());
                        positions.push(index + 1);
                    }
                }
                if cursor != old.len() {
                    return Err(error(
                        "Scripts removed, reordered or replaced existing rules; preview manually",
                    ));
                }
                if remaining.len() > 20 {
                    return Err(error("Scripts added too many rules; preview manually"));
                }
                for value in &remaining {
                    let r = crate::config::MergeConfig {
                        rules: vec![crate::config::MergeRule {
                            key: "rules".into(),
                            op: crate::config::MergeOp::Append {
                                items: vec![value.clone()],
                            },
                        }],
                    };
                    validate_merge(&r)?;
                }
                changes.push(Change {
                    path: key.into(),
                    before: render(before.get(key)),
                    after: remaining
                        .iter()
                        .zip(positions)
                        .map(|(r, pos)| format!("{pos}: {}", r.as_str().unwrap()))
                        .collect::<Vec<_>>()
                        .join("\n"),
                });
            } else {
                for value in [before.get(key), after.get(key)].into_iter().flatten() {
                    let valid = match key {
                        "ipv6" | "unified-delay" => value.is_bool(),
                        "log-level" => value.as_str().is_some_and(|s| {
                            ["silent", "error", "warning", "info", "debug"].contains(&s)
                        }),
                        _ => false,
                    };
                    if !valid {
                        return Err(error(
                            "Scripts produced unsupported values; preview manually",
                        ));
                    }
                }
                changes.push(Change {
                    path: key.into(),
                    before: render(before.get(key)),
                    after: render(after.get(key)),
                });
            }
        }
    }
    // A script may change arbitrary fields after the proposed Merge. Never hide those effects.
    let mut other = HashSet::new();
    for value in [&before, &after] {
        if let Some(m) = value.as_mapping() {
            for key in m.keys().filter_map(|v| v.as_str()) {
                if !["rules", "ipv6", "unified-delay", "log-level"].contains(&key)
                    && before.get(key) != after.get(key)
                {
                    other.insert(key.to_owned());
                }
            }
        }
    }
    if !other.is_empty() {
        return Err(error(
            "Scripts changed additional fields; edit and preview this configuration manually",
        ));
    }
    Ok(changes)
}

/// Validation never runs an enabled core. Bound its lifetime and discard raw error text.
pub(crate) fn validate(
    binary: &Path,
    path: &Path,
    working: &Path,
    cancel: &AtomicBool,
) -> Result<(), AppError> {
    use std::process::{Command, Stdio};
    if cancel.load(Ordering::Relaxed) {
        return Err(error("Cancelled"));
    }
    let isolated = path
        .parent()
        .ok_or_else(|| error("Invalid validation path"))?
        .join("validation-data");
    std::fs::create_dir_all(&isolated).map_err(|_| error("Unable to prepare validation"))?;
    for name in [
        "geoip.dat",
        "geosite.dat",
        "geoip.metadb",
        "country.mmdb",
        "Country.mmdb",
        "GeoLite2-ASN.mmdb",
        "ASN.mmdb",
    ] {
        if cancel.load(Ordering::Relaxed) {
            return Err(error("Cancelled"));
        }
        if working.join(name).is_file() && !isolated.join(name).exists() {
            let bytes = crate::config::read_bounded(&working.join(name), 128 * 1024 * 1024)
                .map_err(|_| error("Validation data unavailable"))?;
            crate::config::restore_runtime_artifact(&isolated.join(name), &bytes)?;
        }
    }
    let mut child = Command::new(binary)
        .args(["-t", "-d"])
        .arg(&isolated)
        .arg("-f")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| error("Unable to start Mihomo validation"))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err(error("Mihomo rejected the proposed configuration"))
                };
            }
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error("Mihomo validation failed"));
            }
        }
        if cancel.load(Ordering::Relaxed) || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error("Mihomo validation cancelled or timed out"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub(super) fn schema() -> Value {
    let mut entries = tools::schema().as_array().unwrap().clone();
    for (name, description, properties, required) in [
        (
            "test_nodes",
            "Test at most six listed nodes per turn with one fixed HTTPS latency probe. Never changes selection.",
            json!({"group":{"type":"string"},"nodes":{"type":"array","items":{"type":"string"},"minItems":1,"maxItems":6}}),
            json!(["group", "nodes"]),
        ),
        (
            "explain_rules",
            "Explain static domain rules for a user-specified domain; unsupported rule types prevent a definitive routing claim.",
            json!({"host":{"type":"string"}}),
            json!(["host"]),
        ),
        (
            "suggest_node",
            "Prepare a node change for separate UI confirmation; no change occurs now.",
            json!({"group":{"type":"string"},"node":{"type":"string"}}),
            json!(["group", "node"]),
        ),
        (
            "suggest_mode",
            "Prepare a mode change for separate UI confirmation; no change occurs now.",
            json!({"mode":{"type":"string","enum":["rule","global","direct"]}}),
            json!(["mode"]),
        ),
        (
            "preview_merge",
            "Append structured operations to global Merge and validate all profiles. Supports rules prepend/append with DOMAIN, DOMAIN-SUFFIX, DOMAIN-KEYWORD strings, or override boolean ipv6/unified-delay, or log-level. No scripts, listeners, DNS, providers or credentials. Requires separate user confirmation to apply.",
            json!({"merge":{"type":"object","properties":{"rules":{"type":"array","items":{"type":"object","properties":{"key":{"type":"string"},"op":{"type":"string","enum":["prepend","append","override"]},"items":{"type":"array","items":{"type":"string"}},"value":{}},"required":["key","op"],"additionalProperties":false},"maxItems":8}},"required":["rules"],"additionalProperties":false}}),
            json!(["merge"]),
        ),
    ] {
        entries.push(json!({"type":"function","function":{"name":name,"description":description,"parameters":{"type":"object","properties":properties,"required":required,"additionalProperties":false}}}));
    }
    entries.extend(network::schemas());
    entries.extend(proxy::schemas());
    Value::Array(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merge_tools_reject_permissions_scripts_and_hidden_effects() {
        for yaml in [
            "rules: [{key: tun, op: override, value: {enable: true}}]",
            "rules: [{key: external-controller, op: override, value: '0.0.0.0:9090'}]",
            "rules: [{key: rules, op: prepend, items: ['RULE-SET,private,DIRECT']}]",
            "rules: [{key: dns, op: override, value: {enable: true}}]",
        ] {
            assert!(validate_merge(&serde_yaml::from_str(yaml).unwrap()).is_err());
        }
        assert!(
            diff(
                "rules: ['DOMAIN,a.test,DIRECT','MATCH,DIRECT']",
                "rules: ['MATCH,DIRECT','DOMAIN,a.test,DIRECT']"
            )
            .is_err()
        );
        assert!(diff("log-level: info", "log-level: secret-value").is_err());
        assert!(diff("ipv6: false", "ipv6: {password: private}").is_err());
        assert!(parse::<Node>(r#"{"group":"g","node":"n","approval":true}"#).is_err());
        assert!(validated_host("https://example.com/private?token=secret").is_err());
        assert!(
            diff(
                "rules: ['MATCH,DIRECT']\nport: 7890",
                "rules: ['DOMAIN,a.test,DIRECT','MATCH,DIRECT']\nport: 7891"
            )
            .is_err()
        );
        assert!(diff("rules: ['MATCH,DIRECT']", "rules: ['DOMAIN,a.test,DIRECT']").is_err());
        let changes = diff(
            "rules: ['MATCH,DIRECT']",
            "rules: ['DOMAIN,a.test,DIRECT','MATCH,DIRECT']",
        )
        .unwrap();
        assert!(changes[0].after.contains("a.test"));
        assert!(!changes[0].after.contains("MATCH"));
    }
}
