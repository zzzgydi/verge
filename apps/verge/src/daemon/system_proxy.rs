use super::*;
use crate::domain::{SystemProxySettings, SystemProxyTarget};
use crate::platform::PacServer;

#[derive(Default)]
pub(super) struct ProxySession {
    pub target: Option<SystemProxyTarget>,
    pub pac: Option<PacServer>,
    pub last_guard: Option<Instant>,
}
impl Backend {
    fn proxy_target(
        &mut self,
        settings: &SystemProxySettings,
    ) -> Result<SystemProxyTarget, AppError> {
        if self.engine.is_none() {
            return Err(self.runtime_unavailable());
        }
        let selected = self
            .profiles
            .selected()
            .ok_or_else(|| self.runtime_unavailable())?;
        let port = self.profiles.system_proxy_mixed_endpoint(selected)?.port;
        if settings.pac_mode {
            if self.proxy_session.pac.is_none() {
                self.proxy_session.pac = Some(PacServer::start(settings.render_pac(port))?);
            }
            Ok(SystemProxyTarget::Pac {
                url: self.proxy_session.pac.as_ref().unwrap().url(),
            })
        } else {
            Ok(SystemProxyTarget::Manual {
                endpoint: settings.endpoint(port)?,
                bypass: settings.effective_bypass(),
            })
        }
    }

    pub(super) fn execute_system_proxy(
        &mut self,
        command: SystemProxyCommand,
    ) -> Result<SystemProxyCommandResult, AppError> {
        let state = match command {
            SystemProxyCommand::SetEnabled { enabled: true } => {
                // All clients use the daemon's current saved host, mixed port and mode.
                let settings = self.settings.get().system_proxy.clone();
                let target = self.proxy_target(&settings)?;
                let state = self.system_proxy.configure(&target)?;
                self.proxy_session.target = Some(target);
                self.refresh_pac_script();
                state
            }
            SystemProxyCommand::SetEnabled { enabled: false }
            | SystemProxyCommand::Disable
            | SystemProxyCommand::RecoverPending => {
                // Stop ownership before restoring, so a later guard tick cannot re-enable.
                self.proxy_session.target = None;
                let state = self.system_proxy.disable()?;
                self.proxy_session.pac = None;
                state
            }
            other => return SystemProxyCommandHandler::new(&mut self.system_proxy).execute(other),
        };
        self.proxy_session.last_guard = Some(Instant::now());
        Ok(SystemProxyCommandResult {
            state,
            summary: "System proxy updated".into(),
        })
    }

    pub(super) fn synchronize_owned_proxy(&mut self) -> Result<(), AppError> {
        let Some(previous) = self.proxy_session.target.clone() else {
            return Ok(());
        };
        let settings = self.settings.get().system_proxy.clone();
        let result: Result<(), AppError> = (|| {
            let target = self.proxy_target(&settings)?;
            if target != previous {
                self.system_proxy.configure(&target)?;
            }
            self.proxy_session.target = Some(target);
            self.refresh_pac_script();
            Ok(())
        })();
        if let Err(mut error) = result {
            // The core has already committed its new listeners. Do not keep guarding
            // an obsolete port; restore the user's pre-Verge settings instead.
            self.proxy_session.target = None;
            if let Err(restore) = self.system_proxy.disable() {
                error
                    .message
                    .push_str(&format!("; proxy recovery failed: {}", restore.message));
            }
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn refresh_pac_script(&self) {
        if let Some(server) = &self.proxy_session.pac
            && let Some(selected) = self.profiles.selected()
            && let Ok(endpoint) = self.profiles.system_proxy_mixed_endpoint(selected)
        {
            server.set_script(self.settings.get().system_proxy.render_pac(endpoint.port));
        }
    }

    pub(super) fn guard_system_proxy(
        &mut self,
    ) -> Option<Result<SystemProxyCommandResult, AppError>> {
        let settings = &self.settings.get().system_proxy;
        if !guard_due(
            settings,
            self.proxy_session.target.is_some(),
            self.engine.is_some(),
            self.proxy_session.last_guard.map(|time| time.elapsed()),
        ) {
            return None;
        }
        self.proxy_session.last_guard = Some(Instant::now());
        let target = self.proxy_session.target.as_ref()?.clone();
        let state = match self.system_proxy.state() {
            Ok(state) => state,
            Err(error) => return Some(Err(error)),
        };
        if target.matches(&state) {
            return None;
        }
        Some(
            self.system_proxy
                .configure(&target)
                .map(|state| SystemProxyCommandResult {
                    state,
                    summary: "System proxy restored by guard".into(),
                }),
        )
    }

    /// Apply settings as one transaction with the native login item and proxy state.
    pub(super) fn persist_system_settings(
        &mut self,
        settings: &ApplicationSettings,
    ) -> Result<(), AppError> {
        settings.validate()?;
        let previous = self.settings.get().clone();
        let previous_login = self.login_item.status().unwrap_or(previous.launch_at_login);
        let login_changed = settings.launch_at_login != previous_login;
        let proxy_changed =
            settings.system_proxy != previous.system_proxy && self.proxy_session.target.is_some();
        let target = if proxy_changed {
            Some(self.proxy_target(&settings.system_proxy)?)
        } else {
            None
        };
        let snapshot = if proxy_changed {
            Some(self.system_proxy.state()?)
        } else {
            None
        };
        if login_changed {
            self.login_item.set_enabled(settings.launch_at_login)?;
        }
        let result = (|| {
            if let Some(target) = &target {
                self.system_proxy.configure(target)?;
            }
            self.settings.update(settings.clone())
        })();
        if let Err(mut error) = result {
            if let Some(snapshot) = snapshot
                && let Err(rollback) = self.system_proxy.restore_snapshot(&snapshot)
            {
                error
                    .message
                    .push_str(&format!("; proxy rollback failed: {}", rollback.message));
            }
            if login_changed && let Err(rollback) = self.login_item.set_enabled(previous_login) {
                error.message.push_str(&format!(
                    "; login item rollback failed: {}",
                    rollback.message
                ));
            }
            return Err(error);
        }
        if let Some(target) = target {
            self.proxy_session.target = Some(target);
        }
        self.refresh_pac_script();
        self.proxy_session.last_guard = Some(Instant::now());
        if settings.global_hotkey != previous.global_hotkey {
            self.queue_hotkey_sync(settings.global_hotkey.as_deref());
        }
        Ok(())
    }
}

fn guard_due(
    settings: &SystemProxySettings,
    owned: bool,
    running: bool,
    elapsed: Option<Duration>,
) -> bool {
    settings.guard_enabled
        && owned
        && running
        && elapsed
            .is_none_or(|elapsed| elapsed >= Duration::from_secs(settings.guard_interval_secs))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proxy_preferences_persist_without_enabling_proxy_or_requiring_a_profile() {
        let (mut backend, directory) = super::super::tests::test_backend("proxy-preferences");
        let mut settings = backend.settings.get().clone();
        settings.system_proxy.host = "localhost".into();
        settings.system_proxy.guard_enabled = true;
        settings.system_proxy.guard_interval_secs = 12;
        settings.system_proxy.bypass = vec!["example.com".into()];
        backend.persist_system_settings(&settings).unwrap();
        assert_eq!(backend.settings.get(), &settings);
        assert!(backend.proxy_session.target.is_none());
        assert!(backend.proxy_session.pac.is_none());
        assert!(!backend.config.recovery_path.exists());
        backend
            .execute_profile(AppCommand::UpdateApplicationSettings {
                settings: ApplicationSettings::default(),
            })
            .unwrap();
        assert_eq!(backend.settings.get().system_proxy, settings.system_proxy);
        let mut invalid = settings.clone();
        invalid.system_proxy.guard_interval_secs = 0;
        assert!(backend.persist_system_settings(&invalid).is_err());
        assert_eq!(backend.settings.get(), &settings);
        drop(backend);
        let store = crate::config::FileSettingsStore::open(&directory).unwrap();
        assert_eq!(store.get(), &settings);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn guard_requires_explicit_ownership_running_core_and_elapsed_interval() {
        let mut settings = SystemProxySettings {
            guard_enabled: true,
            ..Default::default()
        };
        assert!(guard_due(
            &settings,
            true,
            true,
            Some(Duration::from_secs(30))
        ));
        assert!(!guard_due(
            &settings,
            true,
            true,
            Some(Duration::from_secs(29))
        ));
        assert!(!guard_due(&settings, false, true, None));
        assert!(!guard_due(&settings, true, false, None));
        settings.guard_enabled = false;
        assert!(!guard_due(&settings, true, true, None));
    }
}
