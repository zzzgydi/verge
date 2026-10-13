//! Local, bounded network checks. Models explain reports; they never authorize changes.
mod checks;
mod service;
#[cfg(test)]
mod tests;
pub(crate) use checks::Context;
#[cfg(test)]
pub(crate) use checks::proxy_severity;
pub use service::Service;

use crate::domain::{AppError, ErrorCode};
use serde::{Deserialize, Serialize};

pub const CAPABILITY: &str = "optimization_v3";
pub const PROBES: [&str; 2] = [
    "https://www.gstatic.com/generate_204",
    "https://cp.cloudflare.com/generate_204",
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Command {
    GetState,
    ClearHistory,
    Check,
    Optimize { group: String },
    Cancel,
    Apply { id: String },
    Analyze { report_id: u64 },
    Configure { observe: bool, model_analysis: bool },
    SetActiveProbes { enabled: bool },
    AllowProbesAndCheck,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum Phase {
    #[default]
    Idle,
    Checking,
    Optimizing,
    Analyzing,
    Verifying,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Severity {
    Pass,
    Notice,
    Warning,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Finding {
    pub code: String,
    pub severity: Severity,
    /// Only bounded, local display values; never raw logs/configuration.
    pub detail: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Measurement {
    pub node: String,
    pub target: usize,
    pub delay_ms: Option<u32>,
    pub captured_at: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Plan {
    pub id: String,
    pub group: String,
    pub previous: String,
    pub proposed: String,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Report {
    pub id: u64,
    pub captured_at: u64,
    pub completed_at: u64,
    pub findings: Vec<Finding>,
    pub measurements: Vec<Measurement>,
    pub groups: Vec<String>,
    pub cancelled: bool,
    pub partial: bool,
    pub background: bool,
    pub plan: Option<Plan>,
    pub outcome: Option<String>,
    pub explanation: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Snapshot {
    pub revision: u64,
    pub phase: Phase,
    pub report: Option<Report>,
    pub history: Vec<Report>,
    pub observe: bool,
    pub model_analysis: bool,
    #[serde(default)]
    pub active_probes: bool,
    pub error: Option<String>,
}

impl Snapshot {
    pub fn busy(&self) -> bool {
        self.phase != Phase::Idle
    }
}

pub(crate) fn error(message: &str) -> AppError {
    AppError::new(ErrorCode::Conflict, message)
}

pub(crate) fn now() -> u64 {
    crate::ai::proposals::now()
}

/// Deliberately excludes node/group names, domains, logs and stable identifiers.
pub(crate) fn model_evidence(report: &Report) -> Vec<crate::ai::tools::Evidence> {
    let mut nodes = std::collections::BTreeMap::new();
    let probes: Vec<_> = report
        .measurements
        .iter()
        .map(|m| {
            let next = nodes.len() + 1;
            let node = *nodes.entry(m.node.as_str()).or_insert(next);
            serde_json::json!({"node_index": node, "target": m.target, "delay_ms": m.delay_ms})
        })
        .collect();
    vec![crate::ai::tools::Evidence {
        id: format!("H{}", report.id),
        source: "local_health_report".into(),
        captured_at: report.captured_at,
        data: serde_json::json!({
            "findings": report.findings.iter().map(|f| serde_json::json!({"check":f.code,"result":f.severity})).collect::<Vec<_>>(),
            "probes": probes,
            "partial":report.partial,"cancelled":report.cancelled,
            "outcome": report.outcome,
            "scope":"Fixed HTTPS probes and local checks only. No OS DNS or user website verification. No actions are available."
        }),
    }]
}
