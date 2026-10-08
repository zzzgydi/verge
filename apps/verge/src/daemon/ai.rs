use super::*;
use crate::ai::{
    AiOperation,
    proposals::{self, Action, Pending, RuntimeBaseline},
};
use crate::application::{CoreControl as _, RuntimeControl as _};

/// Raw compiler/controller failures may contain credentials. Only these local outcomes reach AI UI.
#[derive(Debug)]
enum Failure {
    Rejected,
    Restored,
    RecoveryFailed,
}
impl From<AppError> for Failure {
    fn from(_: AppError) -> Self {
        Self::Rejected
    }
}
impl Failure {
    fn status(&self) -> &'static str {
        match self {
            Self::Rejected => "rejected",
            Self::Restored => "restored",
            Self::RecoveryFailed => "recovery_failed",
        }
    }
    fn message(&self) -> &'static str {
        match self {
            Self::Rejected => "State changed or is unavailable; no change applied. Preview again.",
            Self::Restored => "Application failed; the previous state was restored and verified.",
            Self::RecoveryFailed => {
                "Application and recovery failed. Check the core and current configuration before retrying."
            }
        }
    }
}

fn transaction<T>(
    target: &mut T,
    apply: impl FnOnce(&mut T) -> Result<(), AppError>,
    restore: impl FnOnce(&mut T) -> Result<(), AppError>,
) -> Result<(), Failure> {
    if apply(target).is_err() {
        return Err(if restore(target).is_ok() {
            Failure::Restored
        } else {
            Failure::RecoveryFailed
        });
    }
    Ok(())
}

fn runtime_matches(engine: &mut Engine, baseline: &RuntimeBaseline) -> Result<(), AppError> {
    let current = RuntimeBaseline::new(
        engine.runtime.mode()?,
        engine.runtime.network_settings()?,
        engine.runtime.proxy_groups()?,
    );
    if &current == baseline {
        Ok(())
    } else {
        Err(crate::ai::error("Runtime changed"))
    }
}
pub(super) fn restore_selections(
    engine: &mut Engine,
    baseline: &RuntimeBaseline,
) -> Result<(), AppError> {
    engine.runtime.set_mode(baseline.mode)?;
    for group in &baseline.groups {
        if let Some(node) = &group.selected {
            engine.runtime.select_proxy(&group.name, node)?;
        }
    }
    runtime_matches(engine, baseline)
}

impl Backend {
    pub(super) fn approve_ai(&mut self, id: String, digest: String) -> UiResponse {
        let result = (|| {
            // Invalid confirmations leave the original proposal available; consumed ones cannot replay.
            let pending = self.ai.take_proposal(&id, &digest)?;
            let outcome = self
                .audit_ai(&pending, "confirmed")
                .map_err(Failure::from)
                .and_then(|()| self.apply_ai(&pending));
            let (status, message) = match &outcome {
                Ok(()) => ("applied", "Applied and verified"),
                Err(failure) => (failure.status(), failure.message()),
            };
            let recorded = self.audit_ai(&pending, status).is_ok();
            let message = if recorded {
                message.to_owned()
            } else {
                format!("{message} The audit result could not be saved.")
            };
            let snapshot = self.ai.finish_proposal(&id, status, &message);
            outcome
                .map(|()| snapshot)
                .map_err(|_| crate::ai::error(&message))
        })();
        UiResponse::Ai {
            operation: AiOperation::Approve,
            result,
        }
    }

    fn apply_ai(&mut self, pending: &Pending) -> Result<(), Failure> {
        if self.profiles.preview_digest()? != pending.baseline {
            return Err(Failure::Rejected);
        }
        match &pending.action {
            Action::Mode { mode, previous } => {
                let engine = self.engine.as_mut().ok_or(Failure::Rejected)?;
                if engine.runtime.mode()? != *previous {
                    return Err(Failure::Rejected);
                }
                transaction(
                    &mut engine.runtime,
                    |runtime| {
                        runtime.set_mode(*mode)?;
                        if runtime.mode()? == *mode {
                            Ok(())
                        } else {
                            Err(crate::ai::error("Verification failed"))
                        }
                    },
                    |runtime| {
                        runtime.set_mode(*previous)?;
                        if runtime.mode()? == *previous {
                            Ok(())
                        } else {
                            Err(crate::ai::error("Verification failed"))
                        }
                    },
                )?;
            }
            Action::Select {
                group,
                proxy,
                previous,
            } => {
                let engine = self.engine.as_mut().ok_or(Failure::Rejected)?;
                let snapshot = engine.runtime.proxy_groups()?;
                let current = snapshot
                    .groups
                    .iter()
                    .find(|g| g.name == *group && g.kind.eq_ignore_ascii_case("selector"))
                    .ok_or(Failure::Rejected)?;
                if current.selected.as_ref() != Some(previous) || !current.members.contains(proxy) {
                    return Err(Failure::Rejected);
                }
                let verify = |runtime: &mut MihomoRuntime<TcpControllerTransport>,
                              expected: &str| {
                    if runtime
                        .proxy_groups()?
                        .groups
                        .iter()
                        .any(|g| g.name == *group && g.selected.as_deref() == Some(expected))
                    {
                        Ok(())
                    } else {
                        Err(crate::ai::error("Verification failed"))
                    }
                };
                transaction(
                    &mut engine.runtime,
                    |runtime| {
                        runtime.select_proxy(group, proxy)?;
                        verify(runtime, proxy)
                    },
                    |runtime| {
                        runtime.select_proxy(group, previous)?;
                        verify(runtime, previous)
                    },
                )?;
            }
            Action::Merge {
                yaml,
                preview,
                selected,
                candidate,
                runtime_digest,
                previous_runtime,
                runtime,
            } => {
                if self.profiles.selected() != Some(selected) {
                    return Err(Failure::Rejected);
                }
                runtime_matches(self.engine.as_mut().ok_or(Failure::Rejected)?, runtime)?;
                let previous_merge = self.profiles.merge_yaml()?;
                let path = self.config.data_dir.join("profiles/runtime-config.yaml");
                let old_runtime = self.profiles.runtime_bytes()?;
                if proposals::digest(&old_runtime) != *previous_runtime
                    || proposals::digest(candidate) != *runtime_digest
                {
                    return Err(Failure::Rejected);
                }
                // The worker already compiled and validated every profile. Copy its exact artifacts;
                // confirmation never reruns scripts or substitutes a newly generated candidate.
                self.profiles.copy_compilation_cache_from(&preview.store)?;
                if self.profiles.preview_digest()? != pending.baseline {
                    return Err(Failure::Rejected);
                }
                transaction(
                    self,
                    |backend| {
                        backend.profiles.set_merge_yaml(yaml)?;
                        crate::config::restore_runtime_artifact(&path, candidate)?;
                        let engine = backend.engine.as_mut().unwrap();
                        let mut core =
                            SupervisorControl::new(&mut engine.supervisor, Duration::from_secs(5));
                        core.apply_config(&path)?;
                        core.health()?;
                        // A config reload must not silently reset the user's runtime choices.
                        let mut expected = runtime.clone();
                        let yaml: serde_yaml::Value = serde_yaml::from_slice(candidate)
                            .map_err(|_| crate::ai::error("Invalid candidate"))?;
                        expected.network.ipv6_enabled = yaml["ipv6"]
                            .as_bool()
                            .unwrap_or(runtime.network.ipv6_enabled);
                        restore_selections(engine, &expected)
                    },
                    |backend| {
                        // Attempt both disk restorations even if one fails.
                        let merge = backend.profiles.set_merge_yaml(&previous_merge);
                        let artifact = crate::config::restore_runtime_artifact(&path, &old_runtime);
                        merge?;
                        artifact?;
                        let engine = backend.engine.as_mut().unwrap();
                        let mut core =
                            SupervisorControl::new(&mut engine.supervisor, Duration::from_secs(5));
                        core.apply_config(&path)?;
                        core.health()?;
                        restore_selections(engine, runtime)
                    },
                )?;
                self.refresh_sensitive_values();
            }
        }
        Ok(())
    }

    fn audit_ai(&self, pending: &Pending, status: &str) -> Result<(), AppError> {
        let path = self.config.data_dir.join("ai-actions.json");
        let mut entries: Vec<serde_json::Value> = crate::config::read_bounded(&path, 64 * 1024)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        entries.push(
            serde_json::json!({"id":pending.view.id,"digest":pending.view.digest,
            "kind":pending.view.kind,"confirmed_at":proposals::now(),"status":status}),
        );
        if entries.len() > 64 {
            entries.drain(..entries.len() - 64);
        }
        let bytes =
            serde_json::to_vec(&entries).map_err(|_| crate::ai::error("Audit unavailable"))?;
        crate::config::restore_runtime_artifact(&path, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_writes_restore_and_recovery_failures_are_distinct() {
        let mut value = 1;
        let result = transaction(
            &mut value,
            |v| {
                *v = 2;
                Err(crate::ai::error("private failure"))
            },
            |v| {
                *v = 1;
                Ok(())
            },
        );
        assert!(matches!(result, Err(Failure::Restored)));
        assert_eq!(value, 1);
        let result = transaction(
            &mut value,
            |v| {
                *v = 2;
                Err(crate::ai::error("private failure"))
            },
            |_| Err(crate::ai::error("private rollback failure")),
        );
        assert!(matches!(result, Err(Failure::RecoveryFailed)));
        assert!(!result.unwrap_err().message().contains("private"));
    }
}
