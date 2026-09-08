use super::*;
use crate::domain::CoreNetworkSettings;

impl<C: CoreControl> ConfigCommandHandler<'_, C> {
    /// Validate before applying; any later failure restores the persisted override and runtime.
    pub fn update_network_settings(
        &mut self,
        settings: CoreNetworkSettings,
        mut after_apply: impl FnMut(&FileProfileStore) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        settings.validate()?;
        let previous = self.profiles.network_override();
        let selected = self.profiles.selected().cloned();
        if selected.is_some() {
            let current = self.profiles.network_settings()?;
            let available = |address: SocketAddr| {
                std::net::TcpListener::bind(address)
                    .map(|_| ())
                    .map_err(|_| {
                        AppError::new(
                            ErrorCode::Conflict,
                            format!("监听地址 {address} 不可用，请选择其他端口"),
                        )
                    })
            };
            if settings.mixed_port != current.mixed_port {
                available(SocketAddr::from(([127, 0, 0, 1], settings.mixed_port)))?;
            }
            let next = &settings.external_controller;
            let old = &current.external_controller;
            if next.enabled && (!old.enabled || old.address != next.address) {
                available(next.address.parse().expect("validated controller address"))?;
            }
        }
        self.profiles.set_network_override(Some(settings))?;
        let mut applied = false;
        let result = (|| {
            for profile in self.profiles.list() {
                self.profiles.merged_yaml(&profile.id)?;
            }
            if let Some(id) = &selected {
                let source = self.profiles.yaml(id)?;
                let candidate = match &self.runtime_credentials {
                    Some(credentials) => self.profiles.prepare_runtime_candidate(
                        id,
                        &source,
                        credentials.controller,
                        &credentials.secret,
                    )?,
                    None => self.profiles.prepare_merge_candidate(id)?,
                };
                self.core.validate_candidate(candidate.path())?;
                let path = self.active_path(id)?;
                applied = true;
                self.core.apply_config(&path)?;
                self.core.health()?;
                after_apply(self.profiles)?;
            }
            Ok(())
        })();
        if let Err(cause) = result {
            let rollback = self.profiles.set_network_override(previous).and_then(|()| {
                if applied && let Some(id) = &selected {
                    let path = self.active_path(id)?;
                    self.core.apply_config(&path)?;
                    self.core.health()?;
                }
                Ok(())
            });
            return match rollback {
                Ok(()) => Err(cause),
                Err(error) => Err(AppError::new(
                    ErrorCode::StorageFailed,
                    format!(
                        "网络设置应用失败：{}；恢复失败：{}",
                        cause.message, error.message
                    ),
                )),
            };
        }
        Ok(())
    }
}
