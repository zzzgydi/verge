use super::*;
use crate::domain::CoreNetworkSettings;
use serde_yaml::Value;

#[derive(serde::Serialize, serde::Deserialize)]
struct NetworkFile {
    settings: CoreNetworkSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    secret_account: Option<String>,
}

pub(super) fn load(
    root: &Path,
    keychain_service: Option<&str>,
) -> Result<Option<CoreNetworkSettings>, AppError> {
    let path = root.join("network-settings.yaml");
    if !path.exists() {
        return Ok(None);
    }
    let mut file: NetworkFile = serde_yaml::from_slice(&fs::read(path).map_err(storage_error)?)
        .map_err(|_| AppError::new(ErrorCode::StorageFailed, "invalid network settings file"))?;
    if let Some(account) = file.secret_account {
        let service = keychain_service.ok_or_else(|| {
            AppError::new(
                ErrorCode::StorageFailed,
                "network settings require the system keychain",
            )
        })?;
        file.settings.external_controller.secret =
            crate::platform::MacKeychain::new(crate::platform::ProcessRunner, service)?
                .get(&account)?;
    }
    file.settings.validate()?;
    Ok(Some(file.settings))
}

impl FileProfileStore {
    pub fn preserve_runtime_tun(&mut self, enabled: bool) {
        self.runtime_tun = Some(enabled);
    }

    pub fn set_internal_socket(&mut self, path: PathBuf) {
        self.internal_socket = Some(path);
    }
    pub fn network_override(&self) -> Option<CoreNetworkSettings> {
        self.network.clone()
    }

    /// Before first save, initialize controls from the selected subscription + Merge.
    pub fn network_settings(&self) -> Result<CoreNetworkSettings, AppError> {
        if let Some(settings) = &self.network {
            return Ok(settings.clone());
        }
        let mut settings = CoreNetworkSettings::default();
        if let Some(id) = self.selected() {
            let value = self.effective_value(id)?;
            settings.allow_lan = value
                .get("allow-lan")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            settings.ipv6 = value.get("ipv6").and_then(Value::as_bool).unwrap_or(false);
            settings.unified_delay = value
                .get("unified-delay")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if let Some(level) = value.get("log-level").and_then(Value::as_str) {
                settings.log_level = level.into();
            }
            if let Ok(endpoint) = self.system_proxy_endpoint(id) {
                settings.mixed_port = endpoint.port;
            }
            if let Some(dns) = value.get("dns").filter(|dns| dns.is_mapping()) {
                settings.dns_yaml = serde_yaml::to_string(dns).map_err(storage_error)?;
            }
        }
        Ok(settings)
    }

    pub fn set_network_override(
        &mut self,
        settings: Option<CoreNetworkSettings>,
    ) -> Result<(), AppError> {
        let path = self.root.join("network-settings.yaml");
        let previous_account = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_yaml::from_slice::<NetworkFile>(&bytes).ok())
            .and_then(|file| file.secret_account);
        let mut next_account = None;
        if let Some(settings) = &settings {
            settings.validate()?;
            let mut file = NetworkFile {
                settings: settings.clone(),
                secret_account: None,
            };
            if let Some(service) = &self.keychain_service {
                let secret = &settings.external_controller.secret;
                if !secret.is_empty() {
                    let existing = previous_account.clone();
                    if self
                        .network
                        .as_ref()
                        .is_some_and(|old| old.external_controller.secret == *secret)
                        && existing.is_some()
                    {
                        file.secret_account = existing;
                    } else {
                        let mut nonce = [0_u8; 16];
                        getrandom::fill(&mut nonce).map_err(|_| {
                            AppError::new(
                                ErrorCode::StorageFailed,
                                "could not generate keychain account",
                            )
                        })?;
                        let account = format!(
                            "controller-{}",
                            nonce.iter().map(|b| format!("{b:02x}")).collect::<String>()
                        );
                        crate::platform::MacKeychain::new(crate::platform::ProcessRunner, service)?
                            .set(&account, secret)?;
                        file.secret_account = Some(account);
                    }
                    file.settings.external_controller.secret.clear();
                }
            }
            let yaml = serde_yaml::to_string(&file).map_err(storage_error)?;
            next_account = file.secret_account;
            if let Err(error) = atomic_write_private(&path, yaml.as_bytes()) {
                if next_account != previous_account {
                    self.remove_secret_account(next_account.as_deref());
                }
                return Err(storage_error(error));
            }
        } else if path.exists() {
            fs::remove_file(path).map_err(storage_error)?;
        }
        if previous_account != next_account {
            self.remove_secret_account(previous_account.as_deref());
        }
        self.network = settings;
        Ok(())
    }

    fn remove_secret_account(&self, account: Option<&str>) {
        if let (Some(service), Some(account)) = (&self.keychain_service, account)
            && let Ok(mut keychain) =
                crate::platform::MacKeychain::new(crate::platform::ProcessRunner, service)
        {
            let _ = keychain.delete(account);
        }
    }

    pub(super) fn apply_network(&self, mut value: Value) -> Result<Value, AppError> {
        if let Some(settings) = &self.network {
            let mapping = value.as_mapping_mut().ok_or_else(|| {
                AppError::new(
                    ErrorCode::ValidationFailed,
                    "profile YAML root must be a mapping",
                )
            })?;
            for (key, value) in [
                ("allow-lan", Value::Bool(settings.allow_lan)),
                ("ipv6", Value::Bool(settings.ipv6)),
                ("unified-delay", Value::Bool(settings.unified_delay)),
                ("log-level", Value::String(settings.log_level.clone())),
                ("mixed-port", Value::Number(settings.mixed_port.into())),
            ] {
                mapping.insert(Value::String(key.into()), value);
            }
            if settings.dns_override {
                // DNS override replaces the whole DNS mapping; disabling it restores the source.
                mapping.insert(
                    Value::String("dns".into()),
                    serde_yaml::from_str(&settings.dns_yaml).map_err(storage_error)?,
                );
            }
        }
        Ok(value)
    }

    pub(super) fn render_runtime(
        &self,
        source: &str,
        controller: SocketAddr,
        secret: &str,
    ) -> Result<String, AppError> {
        let yaml = render_runtime_yaml(source, &self.merge, controller, secret)?;
        let mut value = self.apply_network(serde_yaml::from_str(&yaml).map_err(storage_error)?)?;
        if let Some(enabled) = self.runtime_tun {
            let mapping = value.as_mapping_mut().expect("validated runtime mapping");
            let tun = mapping
                .entry(Value::String("tun".into()))
                .or_insert_with(|| Value::Mapping(Default::default()));
            if let Some(tun) = tun.as_mapping_mut() {
                tun.insert(Value::String("enable".into()), Value::Bool(enabled));
            }
        }
        if let Some(socket) = &self.internal_socket {
            let mapping = value.as_mapping_mut().expect("runtime root was validated");
            // Subscription files cannot add a second unprotected control listener.
            for key in RESERVED_MERGE_KEYS {
                mapping.remove(Value::String(key.into()));
            }
            mapping.insert(
                Value::String("external-controller-unix".into()),
                Value::String(socket.display().to_string()),
            );
            let external = self
                .network
                .as_ref()
                .map(|settings| &settings.external_controller);
            mapping.insert(
                Value::String("external-controller".into()),
                Value::String(
                    external
                        .filter(|c| c.enabled)
                        .map(|c| c.address.clone())
                        .unwrap_or_default(),
                ),
            );
            mapping.insert(
                Value::String("secret".into()),
                Value::String(
                    external
                        .filter(|c| c.enabled)
                        .map(|c| c.secret.clone())
                        .unwrap_or_default(),
                ),
            );
        }
        serde_yaml::to_string(&value).map_err(storage_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests::{TestDir, profile};

    #[test]
    fn network_overrides_merge_after_rules_and_preserve_source() {
        let directory = TestDir::new("network-merge");
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        let id = ProfileId::parse("daily").unwrap();
        let source = "mixed-port: 7890\nipv6: true\ndns:\n  enable: false\n  nameserver: [original]\nrules: [MATCH,DIRECT]\n";
        store
            .import(profile("daily", UpdatePolicy::Manual), source)
            .unwrap();
        store.select(&id).unwrap();
        assert_eq!(store.network_settings().unwrap().mixed_port, 7890);
        assert!(store.network_settings().unwrap().ipv6);
        store
            .set_merge_yaml("rules:\n  - key: mixed-port\n    op: override\n    value: 8000\n")
            .unwrap();
        let mut settings = CoreNetworkSettings {
            mixed_port: 9000,
            dns_override: true,
            ..Default::default()
        };
        store.set_network_override(Some(settings.clone())).unwrap();
        let merged: Value = serde_yaml::from_str(&store.merged_yaml(&id).unwrap()).unwrap();
        assert_eq!(merged["mixed-port"].as_u64(), Some(9000));
        assert_eq!(merged["dns"]["enable"].as_bool(), Some(true));
        assert_eq!(store.system_proxy_endpoint(&id).unwrap().port, 9000);
        assert_eq!(
            store
                .system_proxy_socks_endpoint(&id)
                .unwrap()
                .unwrap()
                .port,
            9000
        );
        assert_eq!(store.yaml(&id).unwrap(), source);
        settings.dns_override = false;
        store.set_network_override(Some(settings.clone())).unwrap();
        let reopened = FileProfileStore::open(&directory.0).unwrap();
        assert_eq!(reopened.network_settings().unwrap(), settings);
        let merged: Value = serde_yaml::from_str(&reopened.merged_yaml(&id).unwrap()).unwrap();
        assert_eq!(merged["dns"]["nameserver"][0].as_str(), Some("original"));
    }

    #[test]
    fn runtime_uses_single_private_file_and_controls_external_listeners() {
        use std::os::unix::fs::PermissionsExt;
        let directory = TestDir::new("runtime-network");
        let mut store = FileProfileStore::open(&directory.0).unwrap();
        let id = ProfileId::parse("daily").unwrap();
        store.import(profile("daily", UpdatePolicy::Manual), "external-controller: 0.0.0.0:9999\nexternal-controller-tls: 0.0.0.0:9998\nsecret: source-secret\n").unwrap();
        store.set_internal_socket(PathBuf::from("/tmp/private/control.sock"));
        let path = store
            .materialize_runtime(&id, "127.0.0.1:12345".parse().unwrap(), "private")
            .unwrap();
        assert_eq!(path.file_name().unwrap(), "runtime-config.yaml");
        let value: Value = serde_yaml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(value["external-controller"].as_str(), Some(""));
        assert!(value.get("external-controller-tls").is_none());
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let mut settings = CoreNetworkSettings::default();
        settings.external_controller.enabled = true;
        settings.external_controller.secret = "test-external-secret".into();
        store.set_network_override(Some(settings)).unwrap();
        assert_eq!(
            store
                .materialize_runtime(&id, "127.0.0.1:12345".parse().unwrap(), "private")
                .unwrap(),
            path
        );
        let value: Value = serde_yaml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            value["external-controller"].as_str(),
            Some("127.0.0.1:9097")
        );
        assert_eq!(value["secret"].as_str(), Some("test-external-secret"));
    }
}
