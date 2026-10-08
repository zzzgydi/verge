use super::*;
use crate::{
    ai::proposals::RuntimeBaseline,
    application::{CoreControl as _, RuntimeControl as _},
    domain::{NetworkSettings, RuntimeCommandResult},
};

impl Backend {
    pub(super) fn set_tun_network(
        &mut self,
        settings: &NetworkSettings,
    ) -> Result<RuntimeCommandResult, AppError> {
        if self.config.channel.is_dev() && settings.tun_enabled {
            return Err(crate::identity::dev_restriction());
        }
        // Turning off must remain possible when the controller is already dead.
        if !settings.tun_enabled && self.tun_lease.is_some() {
            self.disable_tun()?;
        } else {
            let engine = self
                .engine
                .as_mut()
                .ok_or_else(|| AppError::new(ErrorCode::CoreUnavailable, "Core is not running"))?;
            let current = engine.runtime.network_settings()?;
            if settings.tun_enabled && self.tun_lease.is_none() {
                self.enable_tun(settings)?;
            } else if settings != &current {
                if self.tun_lease.is_some() {
                    return Err(AppError::new(
                        ErrorCode::Conflict,
                        "Disable TUN before changing DNS or IPv6",
                    ));
                }
                engine.runtime.set_network_settings(settings)?;
            }
        }
        let actual_result = self
            .engine
            .as_mut()
            .ok_or_else(|| AppError::new(ErrorCode::CoreUnavailable, "Core is not running"))?
            .runtime
            .network_settings();
        let actual = match actual_result {
            Ok(actual) if actual.tun_enabled == settings.tun_enabled => actual,
            result => {
                if self.tun_lease.is_some() {
                    let _ = self.disable_tun();
                }
                return Err(result.err().unwrap_or_else(|| {
                    AppError::new(ErrorCode::PlatformFailed, "TUN state verification failed")
                }));
            }
        };
        Ok(RuntimeCommandResult {
            output: RuntimeCommandOutput::NetworkSettings(actual),
            summary: "Network settings changed and verified".into(),
        })
    }

    fn tun_baseline(&mut self) -> Result<RuntimeBaseline, AppError> {
        let engine = self
            .engine
            .as_mut()
            .ok_or_else(|| AppError::new(ErrorCode::CoreUnavailable, "Core is not running"))?;
        Ok(RuntimeBaseline::new(
            engine.runtime.mode()?,
            engine.runtime.network_settings()?,
            engine.runtime.proxy_groups()?,
        ))
    }

    fn enable_tun(&mut self, settings: &NetworkSettings) -> Result<(), AppError> {
        let selected = self
            .profiles
            .selected()
            .cloned()
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, "Select a profile first"))?;
        let mut baseline = self.tun_baseline()?;
        if settings.ipv6_enabled != baseline.network.ipv6_enabled
            || settings.dns_enabled != baseline.network.dns_enabled
        {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Save DNS and IPv6 settings before enabling TUN",
            ));
        }
        let path = self.config.data_dir.join("profiles/runtime-config.yaml");
        let previous = self.profiles.runtime_bytes()?;
        // Compile and validate all constraints before asking the helper for a device.
        self.profiles
            .set_tun_device(Some(("utun0".into(), settings.ipv6_enabled)));
        let candidate = self.profiles.yaml(&selected).and_then(|yaml| {
            self.profiles.prepare_runtime_candidate(
                &selected,
                &yaml,
                self.config.controller,
                &self.config.secret,
            )
        });
        self.profiles.set_tun_device(None);
        let candidate = candidate?;
        self.engine
            .as_ref()
            .unwrap()
            .supervisor
            .validate_candidate(candidate.path())?;
        let mut lease =
            crate::platform::TunLease::prepare(&self.config.helper_socket, settings.ipv6_enabled)?;
        let fd = lease.fd.try_clone().map_err(|_| {
            AppError::new(ErrorCode::PlatformFailed, "Cannot inherit TUN descriptor")
        })?;
        self.profiles
            .set_tun_device(Some((lease.device.clone(), settings.ipv6_enabled)));
        self.engine
            .as_mut()
            .unwrap()
            .supervisor
            .set_tun_fd(Some(fd));
        baseline.network.tun_enabled = true;
        let applied = (|| {
            let candidate = self.profiles.materialize_runtime(
                &selected,
                self.config.controller,
                &self.config.secret,
            )?;
            let engine = self.engine.as_mut().unwrap();
            let mut core = SupervisorControl::new(&mut engine.supervisor, Duration::from_secs(5));
            core.validate_candidate(&candidate)?;
            core.apply_config(&candidate)?;
            core.health()?;
            super::ai::restore_selections(engine, &baseline)?;
            // A controller can be healthy even when the TUN listener failed.
            engine.runtime.verify_tun(&lease.device)?;
            lease.activate()
        })();
        if let Err(cause) = applied {
            self.profiles.set_tun_device(None);
            let engine = self.engine.as_mut().unwrap();
            let stopped = engine.supervisor.stop();
            engine.supervisor.set_tun_fd(None);
            let released = lease.release();
            baseline.network.tun_enabled = false;
            let restored = (|| {
                crate::config::restore_runtime_artifact(&path, &previous)?;
                let mut core =
                    SupervisorControl::new(&mut engine.supervisor, Duration::from_secs(5));
                core.apply_config(&path)?;
                core.health()?;
                super::ai::restore_selections(engine, &baseline)
            })();
            return Err(AppError::new(
                cause.code,
                format!(
                    "TUN enable failed: {}; recovery {}",
                    cause.message,
                    if stopped.is_ok() && released.is_ok() && restored.is_ok() {
                        "verified"
                    } else {
                        "incomplete; check helper and core"
                    }
                ),
            ));
        }
        self.tun_lease = Some(lease);
        Ok(())
    }

    fn disable_tun(&mut self) -> Result<(), AppError> {
        let baseline = self.tun_baseline().ok();
        // Cleanup is unconditional. A missing profile, dead controller or stop error
        // must never prevent closing both fd copies and the helper connection.
        let stopped = self
            .engine
            .as_mut()
            .map(|engine| {
                let result = engine.supervisor.stop();
                engine.supervisor.set_tun_fd(None);
                result
            })
            .transpose();
        self.profiles.set_tun_device(None);
        let released = self
            .tun_lease
            .take()
            .map(|lease| lease.release())
            .transpose();
        let restarted = (|| {
            if let Some(selected) = self.profiles.selected().cloned() {
                let path = self.profiles.materialize_runtime(
                    &selected,
                    self.config.controller,
                    &self.config.secret,
                )?;
                if let Some(engine) = &mut self.engine {
                    let mut core =
                        SupervisorControl::new(&mut engine.supervisor, Duration::from_secs(5));
                    core.apply_config(&path)?;
                    core.health()?;
                    if let Some(mut baseline) = baseline {
                        baseline.network.tun_enabled = false;
                        super::ai::restore_selections(engine, &baseline)?;
                    }
                }
            }
            Ok::<_, AppError>(())
        })();
        stopped?;
        released?;
        restarted
    }

    pub(super) fn stop_tun_after_failure(&mut self) -> Result<(), AppError> {
        let result = self.disable_tun();
        self.log_app(
            "error",
            "TUN stopped after its helper lease or core was lost",
        );
        result
    }
}
