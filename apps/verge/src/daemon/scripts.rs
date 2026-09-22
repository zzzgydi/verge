use super::*;
use crate::application::CoreControl;
use crate::domain::ProfileId;

impl Backend {
    pub(super) fn validate_offline_yaml(&self, id: &ProfileId, yaml: &str) -> Result<(), AppError> {
        let candidate = self.profiles.prepare_runtime_candidate(
            id,
            yaml,
            self.config.controller,
            &self.config.secret,
        )?;
        let working_dir = self.config.data_dir.join("mihomo");
        fs::create_dir_all(&working_dir)
            .map_err(|e| AppError::new(ErrorCode::StorageFailed, e.to_string()))?;
        crate::mihomo::validate_config_file(&self.config.binary, &working_dir, candidate.path())
    }

    pub(super) fn validate_profiles_offline(&self) -> Result<(), AppError> {
        for profile in self.profiles.list() {
            self.validate_offline_yaml(&profile.id, &self.profiles.yaml(&profile.id)?)?;
        }
        Ok(())
    }

    pub(super) fn execute_script_command(
        &mut self,
        command: AppCommand,
        now: i64,
    ) -> Result<AppCommandResult, AppError> {
        let output = match command {
            AppCommand::GetProfileScript { id } => {
                AppCommandOutput::ProfileScript(self.profiles.script(id.as_ref())?)
            }
            AppCommand::SaveScriptDraft { id, source } => AppCommandOutput::ProfileScript(
                self.profiles.save_script_draft(id.as_ref(), source)?,
            ),
            AppCommand::PreviewProfileScript {
                id,
                profile,
                source,
            } => AppCommandOutput::ScriptPreview(self.profiles.preview_script(
                id.as_ref(),
                &profile,
                &source,
            )?),
            AppCommand::SetProfileScript { id, source } => {
                if source.is_some() && self.profiles.list().is_empty() {
                    return Err(AppError::new(
                        ErrorCode::InvalidInput,
                        "Import a profile before enabling a script",
                    ));
                }
                let previous = self.profiles.script(id.as_ref())?;
                let mut next = previous.clone();
                if let Some(source) = &source {
                    if source.trim().is_empty() {
                        return Err(AppError::new(ErrorCode::InvalidInput, "Script is empty"));
                    }
                    next.draft = source.clone();
                }
                next.active = source;
                let mut runtime = None;
                self.profiles.set_script(id.as_ref(), &next)?;
                if let Err(cause) = self.apply_profile_changes(id.as_ref(), &mut runtime) {
                    self.profiles.set_script(id.as_ref(), &previous)?;
                    self.restore_script_runtime(runtime)?;
                    return Err(cause);
                }
                AppCommandOutput::ProfileScript(next)
            }
            AppCommand::UpdateProfileDetails {
                id,
                name,
                source,
                update_policy,
                user_agent,
            } => {
                let previous = self
                    .profiles
                    .list()
                    .iter()
                    .find(|p| p.id == id)
                    .cloned()
                    .ok_or_else(|| AppError::new(ErrorCode::NotFound, "Profile not found"))?;
                let name_affects_script = previous.name != name.trim()
                    && (self.profiles.script(None)?.active.is_some()
                        || self.profiles.script(Some(&id))?.active.is_some());
                let mut runtime = None;
                self.profiles
                    .update_details(&id, name, source, update_policy, user_agent, now)?;
                if name_affects_script
                    && let Err(cause) = self.apply_profile_changes(Some(&id), &mut runtime)
                {
                    self.profiles.restore_profile_details(previous)?;
                    self.restore_script_runtime(runtime)?;
                    return Err(cause);
                }
                self.refresh_sensitive_values();
                AppCommandOutput::Profiles {
                    profiles: self.profiles.list().to_vec(),
                    selected: self.profiles.selected().cloned(),
                }
            }
            _ => {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "Invalid script command",
                ));
            }
        };
        Ok(AppCommandResult {
            output,
            summary: "Profile changes completed".into(),
        })
    }

    fn apply_profile_changes(
        &mut self,
        scope: Option<&ProfileId>,
        rollback_runtime: &mut Option<Vec<u8>>,
    ) -> Result<(), AppError> {
        for profile in self.profiles.list() {
            if scope.is_none_or(|id| &profile.id == id) {
                self.validate_offline_yaml(&profile.id, &self.profiles.yaml(&profile.id)?)?;
            }
        }
        if let Some(selected) = self.profiles.selected().cloned()
            && scope.is_none_or(|id| id == &selected)
            && self.engine.is_some()
        {
            let previous = self.runtime_snapshot()?;
            let path = self.profiles.materialize_runtime(
                &selected,
                self.config.controller,
                &self.config.secret,
            )?;
            // Validation and inactive-profile edits leave the running core untouched.
            // Restore it only once materialization has replaced its configuration.
            *rollback_runtime = previous;
            let engine = self.engine.as_mut().expect("engine was checked above");
            let mut core = SupervisorControl::new(&mut engine.supervisor, Duration::from_secs(5));
            core.apply_config(&path)?;
            core.health()?;
            self.synchronize_owned_proxy()?;
        }
        Ok(())
    }

    fn runtime_snapshot(&self) -> Result<Option<Vec<u8>>, AppError> {
        if self.engine.is_none() {
            return Ok(None);
        }
        fs::read(self.config.data_dir.join("profiles/runtime-config.yaml"))
            .map(Some)
            .map_err(|e| AppError::new(ErrorCode::StorageFailed, e.to_string()))
    }

    fn restore_script_runtime(&mut self, previous: Option<Vec<u8>>) -> Result<(), AppError> {
        if let Some(bytes) = previous
            && let Some(engine) = self.engine.as_mut()
        {
            let path = self.config.data_dir.join("profiles/runtime-config.yaml");
            crate::config::restore_runtime_artifact(&path, &bytes)?;
            let mut core = SupervisorControl::new(&mut engine.supervisor, Duration::from_secs(5));
            core.apply_config(&path)?;
            core.health()?;
            self.synchronize_owned_proxy()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ProfileSource, UpdatePolicy};

    #[test]
    fn offline_edits_validate_and_failed_script_activation_restores_previous_version() {
        let directory = crate::config::tests::TestDir::new("offline-script");
        let config = BackendConfig {
            channel: crate::identity::AppChannel::Dev,
            binary: "/usr/bin/true".into(),
            manifest: directory.0.join("unused-manifest"),
            controller: "127.0.0.1:9090".parse().unwrap(),
            secret: "test".into(),
            services: vec![],
            recovery_path: directory.0.join("recovery.json"),
            helper_socket: directory.0.join("helper.sock"),
            data_dir: directory.0.clone(),
        };
        let mut backend = Backend::new(config).unwrap();
        let id = ProfileId::parse("test").unwrap();
        backend
            .execute_profile(AppCommand::ImportProfile {
                id: id.clone(),
                name: "Test".into(),
                yaml: "mode: rule\n".into(),
                source: ProfileSource::Local,
                update_policy: UpdatePolicy::Manual,
            })
            .unwrap();
        backend
            .execute_profile(AppCommand::UpdateProfileYaml {
                id: id.clone(),
                yaml: "mode: global\n".into(),
            })
            .unwrap();
        assert_eq!(backend.profiles.yaml(&id).unwrap(), "mode: global\n");
        assert!(backend.engine.is_none());
        let source = "function main(c){c.test=1;return c}";
        backend
            .execute_profile(AppCommand::SetProfileScript {
                id: Some(id.clone()),
                source: Some(source.into()),
            })
            .unwrap();
        backend
            .execute_profile(AppCommand::SaveScriptDraft {
                id: Some(id.clone()),
                source: "bad draft".into(),
            })
            .unwrap();
        assert!(
            backend
                .execute_profile(AppCommand::SetProfileScript {
                    id: Some(id.clone()),
                    source: Some("throw new Error('private-subscription')".into())
                })
                .is_err()
        );
        let saved = backend.profiles.script(Some(&id)).unwrap();
        assert_eq!(saved.active.as_deref(), Some(source));
        assert_eq!(saved.draft, "bad draft");
        assert!(
            backend
                .execute_profile(AppCommand::UpdateProfileYaml {
                    id: id.clone(),
                    yaml: "[invalid".into()
                })
                .is_err()
        );
        assert_eq!(backend.profiles.yaml(&id).unwrap(), "mode: global\n");
        backend.config.binary = "/usr/bin/false".into();
        assert!(
            backend
                .execute_profile(AppCommand::UpdateProfileYaml {
                    id: id.clone(),
                    yaml: "mode: direct\n".into()
                })
                .is_err()
        );
        assert_eq!(backend.profiles.yaml(&id).unwrap(), "mode: global\n");
    }
}
