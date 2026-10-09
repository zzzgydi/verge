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

fn validate_platform_action(dev_mode: bool, action: &Action) -> Result<(), Failure> {
    if dev_mode
        && matches!(
            action,
            Action::SystemProxy { .. }
                | Action::ProxySettings { .. }
                | Action::Tun { enabled: true, .. }
        )
    {
        return Err(Failure::Rejected);
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
        validate_platform_action(self.config.channel.is_dev(), &pending.action)?;
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
            Action::Tun { enabled, previous } => {
                self.check_dev_request(&UiRequest::Runtime(RuntimeCommand::SetNetworkSettings {
                    settings: crate::domain::NetworkSettings {
                        tun_enabled: *enabled,
                        ..previous.network.clone()
                    },
                }))?;
                runtime_matches(self.engine.as_mut().ok_or(Failure::Rejected)?, previous)?;
                if previous.network.tun_enabled != self.tun_lease.is_some() {
                    return Err(Failure::Rejected);
                }
                let mut next = previous.network.clone();
                next.tun_enabled = *enabled;
                transaction(
                    self,
                    |backend| {
                        backend.set_tun_network(&next)?;
                        let mut expected = previous.clone();
                        expected.network = next;
                        runtime_matches(
                            backend
                                .engine
                                .as_mut()
                                .ok_or_else(|| crate::ai::error("Core unavailable"))?,
                            &expected,
                        )
                    },
                    |backend| {
                        backend.set_tun_network(&previous.network)?;
                        restore_selections(
                            backend
                                .engine
                                .as_mut()
                                .ok_or_else(|| crate::ai::error("Core unavailable"))?,
                            previous,
                        )
                    },
                )?;
            }
            Action::SystemProxy {
                enabled,
                previous,
                settings_digest,
                saved_enabled,
                owned,
            } => {
                let command = SystemProxyCommand::SetEnabled { enabled: *enabled };
                self.check_dev_request(&UiRequest::SystemProxy(command.clone()))?;
                if self.system_proxy.state()? != *previous
                    || proposals::digest(
                        serde_json::to_vec(&self.settings.get().system_proxy).unwrap(),
                    ) != *settings_digest
                    || self.settings.system_proxy_enabled() != *saved_enabled
                    || self.proxy_session.target.is_some() != *owned
                {
                    return Err(Failure::Rejected);
                }
                // Reuse the normal proxy transaction and persisted startup intent. A failed
                // cleanup must retain its recovery record and must not silently re-enable.
                let result = self.execute_system_proxy(command).and_then(|result| {
                    let actual = self.system_proxy.state()?;
                    if actual != result.state
                        || self.settings.system_proxy_enabled() != *enabled
                        || (*enabled
                            && !self
                                .proxy_session
                                .target
                                .as_ref()
                                .is_some_and(|target| target.matches(&actual)))
                        || (!*enabled
                            && (self.proxy_session.target.is_some() || actual.recovery_pending))
                    {
                        return Err(crate::ai::error("Proxy verification failed"));
                    }
                    Ok(())
                });
                if result.is_err() {
                    if *enabled
                        && !previous.recovery_pending
                        && (self.proxy_session.target.is_some()
                            || self.system_proxy.recovery_pending())
                    {
                        // An enable that fails verification must release the new ownership
                        // and restore the saved OS snapshot. Disables keep explicit off intent.
                        let cleanup = self.execute_system_proxy(SystemProxyCommand::SetEnabled {
                            enabled: false,
                        });
                        if cleanup.is_ok() {
                            let _ = self.settings.set_system_proxy_enabled(*saved_enabled);
                        }
                    }
                    return Err(
                        if self
                            .system_proxy
                            .state()
                            .is_ok_and(|state| state == *previous)
                            && self.settings.system_proxy_enabled() == *saved_enabled
                            && self.proxy_session.target.is_some() == *owned
                        {
                            Failure::Restored
                        } else {
                            Failure::RecoveryFailed
                        },
                    );
                }
            }
            Action::ProxySettings {
                previous,
                settings,
                state,
                saved_enabled,
            } => {
                if self.settings.get().system_proxy.as_ref() != previous.as_ref()
                    || self.settings.system_proxy_enabled() != *saved_enabled
                    || self.proxy_session.target.is_some() != state.is_some()
                {
                    return Err(Failure::Rejected);
                }
                if let Some(state) = state
                    && self.system_proxy.state()? != *state
                {
                    return Err(Failure::Rejected);
                }
                self.update_system_proxy_settings(settings)
                    .map_err(|failure| match failure {
                        super::system_proxy::PreferenceFailure::Rejected(_) => Failure::Rejected,
                        super::system_proxy::PreferenceFailure::Restored(_) => Failure::Restored,
                        super::system_proxy::PreferenceFailure::RecoveryFailed(_) => {
                            Failure::RecoveryFailed
                        }
                    })?;
            }
            Action::Merge {
                network,
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
                let previous_network = self.profiles.network_override();
                if network.is_some() && (runtime.network.tun_enabled || self.tun_lease.is_some()) {
                    return Err(Failure::Rejected);
                }
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
                        if let Some(settings) = network {
                            backend
                                .profiles
                                .set_network_override(Some(settings.clone()))?;
                        } else {
                            backend.profiles.set_merge_yaml(yaml)?;
                        }
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
                        let merge = if network.is_some() {
                            backend
                                .profiles
                                .set_network_override(previous_network.clone())
                        } else {
                            backend.profiles.set_merge_yaml(&previous_merge)
                        };
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

#[cfg(test)]
mod network_action_tests {
    use super::*;
    #[test]
    fn proxy_settings_tool_previews_without_writes_then_confirms_or_rejects_stale_state() {
        use crate::ai::{
            diagnostics::{ConfigContext, Session},
            tools::{NetworkContext, ToolContext},
        };
        use std::sync::{Arc, atomic::AtomicBool};
        let (mut backend, directory) = super::super::tests::test_backend("ai-bypass-confirm");
        let session = |backend: &Backend| {
            Session::new(
                ToolContext {
                    socket: backend.config.internal_socket(),
                    services: vec![],
                    recovery_path: backend.config.recovery_path.clone(),
                    config_selected: false,
                    connections: None,
                    errors: Default::default(),
                    config: Some(ConfigContext {
                        profiles: backend.profiles.clone(),
                        binary: backend.config.binary.clone(),
                        working_dir: directory.clone(),
                        controller: backend.config.controller,
                        secret: backend.config.secret.clone(),
                    }),
                    network: NetworkContext {
                        proxy_settings: Some((*backend.settings.get().system_proxy).clone()),
                        dev_mode: backend.config.channel.is_dev(),
                        ..Default::default()
                    },
                },
                vec![],
                1,
                backend.ai.proposal_store(),
                Arc::new(AtomicBool::new(false)),
            )
        };
        let mut diagnostics = session(&backend);
        let preferences = diagnostics.execute("system_proxy_settings", "{}").unwrap();
        assert_eq!(preferences["data"]["pac_script_omitted"], true);
        let proposal=diagnostics.execute("preview_system_proxy_settings",r#"{"add_bypass":["*.example.test"],"guard_enabled":true,"guard_interval_secs":15}"#).unwrap();
        assert_eq!(proposal["status"], "awaiting_user_confirmation");
        assert!(backend.settings.get().system_proxy.bypass.is_empty());
        assert!(!directory.join("settings.json").exists());
        let view = diagnostics.views.last().unwrap().clone();
        assert!(!proposal.to_string().contains(&view.digest));
        assert!(matches!(
            backend.approve_ai(view.id.clone(), "forged".into()),
            UiResponse::Ai { result: Err(_), .. }
        ));
        let result = backend.approve_ai(view.id.clone(), view.digest.clone());
        assert!(
            matches!(result, UiResponse::Ai { result: Ok(_), .. }),
            "{result:?}"
        );
        assert_eq!(
            backend.settings.get().system_proxy.bypass,
            vec!["*.example.test"]
        );
        assert!(backend.settings.get().system_proxy.guard_enabled);
        assert!(backend.proxy_session.target.is_none());
        assert!(backend.proxy_session.pac.is_none());
        assert!(!backend.config.recovery_path.exists());
        assert!(!backend.settings.system_proxy_enabled());
        let saved = FileSettingsStore::open(&directory).unwrap();
        assert_eq!(saved.get(), backend.settings.get());
        assert!(matches!(
            backend.approve_ai(view.id, view.digest),
            UiResponse::Ai { result: Err(_), .. }
        ));
        // General settings may change while a suggestion is open; preserve them.
        let mut diagnostics = session(&backend);
        diagnostics
            .execute(
                "preview_system_proxy_settings",
                r#"{"reset_bypass":true,"pac_mode":true}"#,
            )
            .unwrap();
        let view = diagnostics.views.last().unwrap().clone();
        let mut settings = backend.settings.get().clone();
        settings.language = "zh-CN".into();
        backend.settings.update(settings).unwrap();
        assert!(matches!(
            backend.approve_ai(view.id, view.digest),
            UiResponse::Ai { result: Ok(_), .. }
        ));
        assert!(backend.settings.get().system_proxy.pac_mode);
        assert_eq!(backend.settings.get().language, "zh-CN");
        assert!(backend.proxy_session.pac.is_none());
        for change_intent in [false, true] {
            let mut diagnostics = session(&backend);
            diagnostics
                .execute(
                    "preview_system_proxy_settings",
                    r#"{"add_bypass":["stale.test"]}"#,
                )
                .unwrap();
            let view = diagnostics.views.last().unwrap().clone();
            if change_intent {
                backend.settings.set_system_proxy_enabled(true).unwrap();
            } else {
                let mut latest = backend.settings.get().clone();
                latest.system_proxy.guard_interval_secs += 1;
                backend.settings.update(latest).unwrap();
            }
            assert!(matches!(
                backend.approve_ai(view.id, view.digest),
                UiResponse::Ai { result: Err(_), .. }
            ));
            assert!(backend.settings.get().system_proxy.bypass.is_empty());
        }
        backend.config.channel = crate::identity::AppChannel::Dev;
        assert!(
            session(&backend)
                .execute("preview_system_proxy_settings", r#"{"reset_bypass":true}"#)
                .is_err()
        );
        drop(backend);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn ai_confirmation_cannot_bypass_dev_system_restrictions() {
        let runtime = RuntimeBaseline {
            mode: crate::domain::RunMode::Rule,
            network: crate::domain::NetworkSettings::default(),
            groups: vec![],
        };
        assert!(
            validate_platform_action(
                true,
                &Action::Tun {
                    enabled: true,
                    previous: runtime.clone()
                }
            )
            .is_err()
        );
        assert!(
            validate_platform_action(
                true,
                &Action::Tun {
                    enabled: false,
                    previous: runtime
                }
            )
            .is_ok()
        );
        let proxy = Action::SystemProxy {
            enabled: false,
            previous: crate::domain::SystemProxyState {
                services: vec![],
                recovery_pending: false,
            },
            settings_digest: "test".into(),
            saved_enabled: false,
            owned: false,
        };
        assert!(validate_platform_action(true, &proxy).is_err());
        assert!(validate_platform_action(false, &proxy).is_ok());
        let settings = Action::ProxySettings {
            previous: Box::default(),
            settings: Box::default(),
            state: None,
            saved_enabled: false,
        };
        assert!(validate_platform_action(true, &settings).is_err());
    }
}
