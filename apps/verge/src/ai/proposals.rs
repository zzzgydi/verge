//! Models can prepare proposals; only a UI confirmation can consume one.
use super::{error, tools::Evidence};
use crate::{
    config::ConfigPreview,
    domain::{AppError, ProfileId, RunMode},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Change {
    pub path: String,
    pub before: String,
    pub after: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Proposal {
    pub id: String,
    pub run_id: u64,
    pub digest: String,
    pub kind: String,
    pub target: String,
    pub expires_at: u64,
    pub status: String,
    pub changes: Vec<Change>,
    pub evidence_id: String,
    pub result: Option<String>,
}

pub(crate) enum Action {
    Select {
        group: String,
        proxy: String,
        previous: String,
    },
    Mode {
        mode: RunMode,
        previous: RunMode,
    },
    Merge {
        yaml: String,
        preview: Box<ConfigPreview>,
        selected: ProfileId,
        candidate: Vec<u8>,
        runtime_digest: String,
        previous_runtime: String,
        runtime: RuntimeBaseline,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RuntimeBaseline {
    pub mode: RunMode,
    pub network: crate::domain::NetworkSettings,
    pub groups: Vec<crate::domain::ProxyGroup>,
}

impl RuntimeBaseline {
    pub fn new(
        mode: RunMode,
        network: crate::domain::NetworkSettings,
        snapshot: crate::domain::ProxySnapshot,
    ) -> Self {
        let mut groups: Vec<_> = snapshot
            .groups
            .into_iter()
            .filter(|g| g.kind.eq_ignore_ascii_case("selector"))
            .collect();
        groups.sort_by(|a, b| a.name.cmp(&b.name));
        Self {
            mode,
            network,
            groups,
        }
    }
}

pub(crate) struct Pending {
    pub view: Proposal,
    pub action: Action,
    pub baseline: String,
}

#[derive(Default)]
pub(crate) struct Proposals {
    entries: VecDeque<Pending>,
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn digest(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}

impl Proposals {
    #[allow(clippy::too_many_arguments)]
    pub fn insert(
        &mut self,
        run_id: u64,
        action: Action,
        baseline: String,
        kind: &str,
        target: String,
        changes: Vec<Change>,
        evidence: &Evidence,
    ) -> Result<Proposal, AppError> {
        if self
            .entries
            .iter()
            .filter(|e| e.view.run_id == run_id)
            .count()
            >= 4
        {
            return Err(error("At most four proposals per turn"));
        }
        if serde_json::to_vec(&changes).map_or(true, |v| v.len() > 24 * 1024) {
            return Err(error("Proposal preview exceeds 24 KiB"));
        }
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).map_err(|_| error("Unable to create proposal"))?;
        let id = nonce.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let expires_at = now() + 300;
        let action_version = match &action {
            Action::Mode { mode, previous } => format!("{mode:?}/{previous:?}"),
            Action::Select {
                group,
                proxy,
                previous,
            } => format!("{group:?}/{proxy:?}/{previous:?}"),
            Action::Merge {
                yaml,
                selected,
                runtime_digest,
                previous_runtime,
                runtime,
                ..
            } => format!(
                "{}/{selected:?}/{runtime_digest}/{previous_runtime}/{runtime:?}",
                digest(yaml)
            ),
        };
        let digest = digest(
            serde_json::to_vec(&(
                &id,
                run_id,
                kind,
                &target,
                &changes,
                &baseline,
                expires_at,
                action_version,
            ))
            .unwrap(),
        );
        let view = Proposal {
            id,
            run_id,
            digest,
            kind: kind.into(),
            target,
            expires_at,
            status: "pending".into(),
            changes,
            evidence_id: evidence.id.clone(),
            result: None,
        };
        while self.entries.len() >= 8 {
            self.entries.pop_front();
        }
        self.entries.push_back(Pending {
            view: view.clone(),
            action,
            baseline,
        });
        Ok(view)
    }

    pub fn consume(&mut self, id: &str, digest: &str) -> Result<Pending, AppError> {
        let index = self
            .entries
            .iter()
            .position(|e| e.view.id == id)
            .ok_or_else(|| error("Proposal is no longer available; preview again"))?;
        if self.entries[index].view.digest != digest {
            return Err(error("Proposal confirmation does not match"));
        }
        let pending = self.entries.remove(index).unwrap();
        if pending.view.expires_at <= now() {
            return Err(error("Proposal expired; preview again"));
        }
        Ok(pending)
    }
    pub fn dismiss(&mut self, id: &str) {
        self.entries.retain(|p| p.view.id != id);
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confirmation_is_bound_single_use_and_expiring() {
        let mut store = Proposals::default();
        let evidence = Evidence {
            id: "R1-E1".into(),
            source: "runtime_status".into(),
            captured_at: now(),
            data: serde_json::json!({}),
        };
        let mut insert = || {
            store
                .insert(
                    1,
                    Action::Mode {
                        mode: RunMode::Direct,
                        previous: RunMode::Rule,
                    },
                    "baseline".into(),
                    "mode",
                    "direct".into(),
                    vec![],
                    &evidence,
                )
                .unwrap()
        };
        let p = insert();
        assert!(store.consume(&p.id, "forged").is_err());
        assert!(store.consume(&p.id, &p.digest).is_ok());
        assert!(store.consume(&p.id, &p.digest).is_err());
        let p = store
            .insert(
                1,
                Action::Mode {
                    mode: RunMode::Direct,
                    previous: RunMode::Rule,
                },
                "baseline".into(),
                "mode",
                "direct".into(),
                vec![],
                &evidence,
            )
            .unwrap();
        store.entries[0].view.expires_at = 0;
        assert!(store.consume(&p.id, &p.digest).is_err());
    }
}
