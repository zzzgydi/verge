//! Portable settings exclude controller credentials and preserve the receiving
//! machine's controller configuration. Version 1 imports remain supported.
use super::*;
use crate::domain::CoreNetworkSettings;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableNetwork {
    allow_lan: bool,
    ipv6: bool,
    unified_delay: bool,
    log_level: String,
    mixed_port: u16,
    dns_override: bool,
    dns_yaml: String,
}
impl PortableNetwork {
    pub fn from_settings(settings: &CoreNetworkSettings) -> Self {
        Self {
            allow_lan: settings.allow_lan,
            ipv6: settings.ipv6,
            unified_delay: settings.unified_delay,
            log_level: settings.log_level.clone(),
            mixed_port: settings.mixed_port,
            dns_override: settings.dns_override,
            dns_yaml: settings.dns_yaml.clone(),
        }
    }
    pub fn apply_to(&self, current: &CoreNetworkSettings) -> CoreNetworkSettings {
        CoreNetworkSettings {
            allow_lan: self.allow_lan,
            ipv6: self.ipv6,
            unified_delay: self.unified_delay,
            log_level: self.log_level.clone(),
            mixed_port: self.mixed_port,
            dns_override: self.dns_override,
            dns_yaml: self.dns_yaml.clone(),
            external_controller: current.external_controller.clone(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransferFile {
    version: u32,
    settings: ApplicationSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    network: Option<PortableNetwork>,
}

pub fn export_portable_settings(
    settings: &ApplicationSettings,
    network: &CoreNetworkSettings,
) -> Result<Vec<u8>, AppError> {
    settings.validate()?;
    network.validate()?;
    serde_json::to_vec_pretty(&TransferFile {
        version: 2,
        settings: settings.clone(),
        network: Some(PortableNetwork::from_settings(network)),
    })
    .map_err(storage_error)
}

pub fn parse_portable_settings(
    bytes: &[u8],
) -> Result<(ApplicationSettings, Option<PortableNetwork>), AppError> {
    let file: TransferFile = serde_json::from_slice(bytes)
        .map_err(|_| AppError::new(ErrorCode::ValidationFailed, "invalid settings import file"))?;
    if !(file.version == 1 || file.version == 2) || (file.version == 1 && file.network.is_some()) {
        return Err(AppError::new(
            ErrorCode::ValidationFailed,
            "unsupported settings import version",
        ));
    }
    file.settings.validate()?;
    if let Some(network) = &file.network {
        network
            .apply_to(&CoreNetworkSettings::default())
            .validate()?;
    }
    Ok((file.settings, file.network))
}

pub fn portable_settings_preview(
    current: &ApplicationSettings,
    network: &CoreNetworkSettings,
    bytes: &[u8],
) -> Result<SettingsImportPreview, AppError> {
    let (settings, incoming) = parse_portable_settings(bytes)?;
    let mut changes = diff_application_settings(current, &settings);
    if let Some(incoming) = incoming {
        incoming.apply_to(network).validate()?;
        let old =
            serde_json::to_value(PortableNetwork::from_settings(network)).map_err(storage_error)?;
        let new = serde_json::to_value(incoming).map_err(storage_error)?;
        for (field, value) in new.as_object().unwrap() {
            if old.get(field) != Some(value) {
                changes.push(SettingsFieldChange {
                    field: format!("network.{field}"),
                    old: render_json_value(&old[field]),
                    new: render_json_value(value),
                });
            }
        }
    }
    Ok(SettingsImportPreview { settings, changes })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_roundtrip_includes_network_without_exporting_controller_credentials() {
        let settings = ApplicationSettings::default();
        let mut network = CoreNetworkSettings {
            mixed_port: 18789,
            dns_override: true,
            ..Default::default()
        };
        network.external_controller.secret = "private-controller-token".into();
        network.external_controller.enabled = true;
        let bytes = export_portable_settings(&settings, &network).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(!text.contains("private-controller-token"));
        assert!(!text.contains("external_controller"));
        assert!(!text.contains("secret"));
        let (decoded, portable) = parse_portable_settings(&bytes).unwrap();
        assert_eq!(decoded, settings);
        let current = CoreNetworkSettings::default();
        let applied = portable.unwrap().apply_to(&current);
        assert_eq!(applied.mixed_port, 18789);
        assert!(applied.dns_override);
        assert_eq!(applied.external_controller, current.external_controller);
        let preview = portable_settings_preview(&settings, &current, &bytes).unwrap();
        assert!(
            preview
                .changes
                .iter()
                .any(|c| c.field == "network.mixed_port")
        );
        assert!(
            parse_portable_settings(&export_settings_json(&settings).unwrap())
                .unwrap()
                .1
                .is_none()
        );
    }
}
