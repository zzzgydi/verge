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
    /// Called only after the selected core has passed startup and health checks.
    pub(super) fn resume_system_proxy(&mut self) {
        if !should_resume_proxy(
            self.config.channel,
            self.settings.system_proxy_enabled(),
            self.engine.is_some(),
            self.proxy_session.target.is_some(),
        ) {
            return;
        }
        if let Err(error) =
            self.execute_system_proxy(SystemProxyCommand::SetEnabled { enabled: true })
        {
            self.log_app("error", format!("恢复系统代理失败: {}", error.message));
        }
    }

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
        self.check_dev_request(&UiRequest::SystemProxy(command.clone()))?;
        let state = match command {
            SystemProxyCommand::SetEnabled { enabled: true } => {
                // All clients use the daemon's current saved host, mixed port and mode.
                let settings = self.settings.get().system_proxy.clone();
                let target = self.proxy_target(&settings)?;
                let state = match enable_and_remember(
                    &mut self.system_proxy,
                    &mut self.settings,
                    &target,
                ) {
                    Ok(state) => state,
                    Err(error) => {
                        if self.proxy_session.target.is_none() {
                            self.proxy_session.pac = None;
                        }
                        return Err(error);
                    }
                };
                self.proxy_session.target = Some(target);
                self.refresh_pac_script();
                state
            }
            SystemProxyCommand::SetEnabled { enabled: false } | SystemProxyCommand::Disable => {
                // Persist explicit intent before cleanup. Even if restoring the OS
                // fails, the next launch must not re-enable a switch the user turned off.
                disable_and_remember(
                    &mut self.system_proxy,
                    &mut self.settings,
                    &mut self.proxy_session,
                )?
            }
            SystemProxyCommand::RecoverPending => {
                self.proxy_session.target = None;
                let state = self.system_proxy.recover_pending()?;
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
        if self.config.channel.is_dev() {
            if settings.launch_at_login || settings.global_hotkey.is_some() {
                return Err(crate::identity::dev_restriction());
            }
            return self.settings.update(settings.clone());
        }
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

fn should_resume_proxy(
    channel: crate::identity::AppChannel,
    enabled: bool,
    running: bool,
    owned: bool,
) -> bool {
    !channel.is_dev() && enabled && running && !owned
}

fn enable_and_remember<P: crate::platform::SystemProxyPlatform>(
    proxy: &mut PlatformSystemProxy<P>,
    settings: &mut FileSettingsStore,
    target: &SystemProxyTarget,
) -> Result<crate::domain::SystemProxyState, AppError> {
    let had_recovery = proxy.recovery_pending();
    let previous = proxy.state()?;
    let state = proxy.configure(target)?;
    if let Err(mut error) = settings.set_system_proxy_enabled(true) {
        // Restore the exact prior state for an existing session, or release the
        // recovery snapshot created by a first enable that could not be saved.
        let rollback = if had_recovery {
            proxy.restore_snapshot(&previous)
        } else {
            proxy.recover_pending().map(|_| ())
        };
        if let Err(rollback) = rollback {
            error
                .message
                .push_str(&format!("; proxy recovery failed: {}", rollback.message));
        }
        return Err(error);
    }
    Ok(state)
}

fn disable_and_remember<P: crate::platform::SystemProxyPlatform>(
    proxy: &mut PlatformSystemProxy<P>,
    settings: &mut FileSettingsStore,
    session: &mut ProxySession,
) -> Result<crate::domain::SystemProxyState, AppError> {
    settings.set_system_proxy_enabled(false)?;
    // Stop ownership before restoring, so a later guard tick cannot re-enable.
    session.target = None;
    let state = proxy.disable()?;
    session.pac = None;
    Ok(state)
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
    use crate::domain::{
        AutoProxyState, ProxyProtocolState, SystemProxyServiceState, SystemProxyState,
    };
    use crate::platform::SystemProxyPlatform;
    use std::sync::Mutex;

    struct FakeProxy {
        current: SystemProxyState,
        recovery: Option<SystemProxyState>,
        fail_configure: bool,
        fail_disable: bool,
    }
    struct TestProxy(Arc<Mutex<FakeProxy>>);
    impl SystemProxyPlatform for TestProxy {
        fn configure(
            &mut self,
            _: &[String],
            target: &SystemProxyTarget,
        ) -> Result<SystemProxyState, AppError> {
            let mut p = self.0.lock().unwrap();
            if p.fail_configure {
                return Err(AppError::new(ErrorCode::PlatformFailed, "configure failed"));
            }
            if p.recovery.is_none() {
                p.recovery = Some(p.current.clone());
            }
            let s = &mut p.current.services[0];
            match target {
                SystemProxyTarget::Manual { endpoint, bypass } => {
                    for protocol in [&mut s.web, &mut s.secure_web, &mut s.socks] {
                        *protocol = ProxyProtocolState {
                            enabled: true,
                            endpoint: endpoint.clone(),
                        };
                    }
                    s.auto_proxy.enabled = false;
                    s.bypass = bypass.clone();
                }
                SystemProxyTarget::Pac { url } => {
                    s.web.enabled = false;
                    s.secure_web.enabled = false;
                    s.socks.enabled = false;
                    s.auto_proxy = AutoProxyState {
                        enabled: true,
                        url: Some(url.clone()),
                    };
                }
            }
            p.current.recovery_pending = true;
            Ok(p.current.clone())
        }
        fn state(&mut self, _: &[String]) -> Result<SystemProxyState, AppError> {
            Ok(self.0.lock().unwrap().current.clone())
        }
        fn recovery_pending(&self) -> bool {
            self.0.lock().unwrap().recovery.is_some()
        }
        fn restore_snapshot(&mut self, state: &SystemProxyState) -> Result<(), AppError> {
            self.0.lock().unwrap().current = state.clone();
            Ok(())
        }
        fn recover_pending(&mut self) -> Result<SystemProxyState, AppError> {
            let mut p = self.0.lock().unwrap();
            if let Some(old) = p.recovery.take() {
                p.current = old;
            }
            Ok(p.current.clone())
        }
        fn disable(&mut self, _: &[String]) -> Result<SystemProxyState, AppError> {
            if self.0.lock().unwrap().fail_disable {
                return Err(AppError::new(ErrorCode::PlatformFailed, "disable failed"));
            }
            self.recover_pending()
        }
        fn list_network_services(&mut self) -> Result<Vec<String>, AppError> {
            unreachable!()
        }
        fn enable(
            &mut self,
            _: &[String],
            _: &crate::domain::ProxyEndpoint,
        ) -> Result<SystemProxyState, AppError> {
            unreachable!()
        }
        fn set_socks(
            &mut self,
            _: &[String],
            _: bool,
            _: &crate::domain::ProxyEndpoint,
        ) -> Result<SystemProxyState, AppError> {
            unreachable!()
        }
        fn set_auto_proxy(
            &mut self,
            _: &[String],
            _: Option<&str>,
        ) -> Result<SystemProxyState, AppError> {
            unreachable!()
        }
        fn set_bypass(&mut self, _: &[String], _: &[String]) -> Result<SystemProxyState, AppError> {
            unreachable!()
        }
    }
    fn fake_proxy() -> (PlatformSystemProxy<TestProxy>, Arc<Mutex<FakeProxy>>) {
        let protocol = ProxyProtocolState {
            enabled: false,
            endpoint: crate::domain::ProxyEndpoint::new("127.0.0.1", 8080).unwrap(),
        };
        let state = Arc::new(Mutex::new(FakeProxy {
            current: SystemProxyState {
                services: vec![SystemProxyServiceState {
                    service: "Wi-Fi".into(),
                    web: protocol.clone(),
                    secure_web: protocol.clone(),
                    socks: protocol,
                    auto_proxy: AutoProxyState {
                        enabled: false,
                        url: None,
                    },
                    bypass: vec![],
                }],
                recovery_pending: false,
            },
            recovery: None,
            fail_configure: false,
            fail_disable: false,
        }));
        (
            PlatformSystemProxy::new(TestProxy(state.clone()), vec!["Wi-Fi".into()]).unwrap(),
            state,
        )
    }

    #[test]
    fn proxy_switch_is_remembered_across_cleanup_and_reopen() {
        use crate::identity::AppChannel;
        for target in [
            SystemProxyTarget::Manual {
                endpoint: crate::domain::ProxyEndpoint::new("127.0.0.1", 7897).unwrap(),
                bypass: vec![],
            },
            SystemProxyTarget::Pac {
                url: "http://127.0.0.1:30000/proxy.pac".into(),
            },
        ] {
            let dir = crate::config::tests::TestDir::new("proxy-restart");
            let mut settings = FileSettingsStore::open(&dir.0).unwrap();
            let (mut proxy, _) = fake_proxy();
            enable_and_remember(&mut proxy, &mut settings, &target).unwrap();
            assert!(target.matches(&proxy.state().unwrap()));
            // The lifecycle restores OS settings on shutdown without erasing intent.
            proxy.recover_pending().unwrap();
            assert!(!proxy.state().unwrap().unified_enabled());
            let mut settings = FileSettingsStore::open(&dir.0).unwrap();
            assert!(settings.system_proxy_enabled());
            assert!(should_resume_proxy(
                AppChannel::Stable,
                settings.system_proxy_enabled(),
                true,
                false
            ));
            assert!(!should_resume_proxy(AppChannel::Stable, true, false, false));
            assert!(!should_resume_proxy(AppChannel::Dev, true, true, false));
            assert!(!should_resume_proxy(AppChannel::Stable, true, true, true));
            enable_and_remember(&mut proxy, &mut settings, &target).unwrap();
            let mut session = ProxySession {
                target: Some(target),
                ..Default::default()
            };
            disable_and_remember(&mut proxy, &mut settings, &mut session).unwrap();
            assert!(session.target.is_none());
            assert!(
                !FileSettingsStore::open(&dir.0)
                    .unwrap()
                    .system_proxy_enabled()
            );
            assert!(!should_resume_proxy(AppChannel::Stable, false, true, false));
        }
    }

    #[test]
    fn proxy_enable_failures_roll_back_and_failed_disable_stays_off_next_launch() {
        let dir = crate::config::tests::TestDir::new("proxy-switch-failures");
        let mut settings = FileSettingsStore::open(&dir.0).unwrap();
        let (mut proxy, state) = fake_proxy();
        let target = SystemProxyTarget::Pac {
            url: "http://127.0.0.1:30000/proxy.pac".into(),
        };
        state.lock().unwrap().fail_configure = true;
        assert!(enable_and_remember(&mut proxy, &mut settings, &target).is_err());
        assert!(!settings.system_proxy_enabled());
        state.lock().unwrap().fail_configure = false;
        fs::create_dir(dir.0.join("settings.json")).unwrap();
        assert!(enable_and_remember(&mut proxy, &mut settings, &target).is_err());
        assert!(!proxy.state().unwrap().unified_enabled());
        assert!(!proxy.recovery_pending());
        assert!(!settings.system_proxy_enabled());
        fs::remove_dir(dir.0.join("settings.json")).unwrap();
        enable_and_remember(&mut proxy, &mut settings, &target).unwrap();
        state.lock().unwrap().fail_disable = true;
        let mut session = ProxySession {
            target: Some(target),
            ..Default::default()
        };
        assert!(disable_and_remember(&mut proxy, &mut settings, &mut session).is_err());
        assert!(session.target.is_none());
        assert!(
            !FileSettingsStore::open(&dir.0)
                .unwrap()
                .system_proxy_enabled()
        );
    }

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
