use super::{Secret, error};
use crate::domain::AppError;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub base_url: String,
    pub model: String,
    pub timeout_seconds: u16,
    pub max_tool_steps: u8,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            model: String::new(),
            timeout_seconds: 60,
            max_tool_steps: 4,
        }
    }
}

impl ProviderConfig {
    pub fn validated(mut self) -> Result<Self, AppError> {
        self.base_url = self.base_url.trim().trim_end_matches('/').to_owned();
        self.model = self.model.trim().to_owned();
        let url = reqwest::Url::parse(&self.base_url).map_err(|_| error("Invalid AI Base URL"))?;
        let local = is_loopback(&url);
        if self.base_url.len() > 2048
            || !(url.scheme() == "https" || (url.scheme() == "http" && local))
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(error(
                "AI URL requires HTTPS (HTTP is allowed only on loopback), without credentials, query or fragment",
            ));
        }
        if self.model.is_empty()
            || self.model.len() > 200
            || self.model.chars().any(char::is_control)
            || !(5..=120).contains(&self.timeout_seconds)
            || !(1..=8).contains(&self.max_tool_steps)
        {
            return Err(error(
                "Set a model, timeout of 5–120 seconds and tool limit of 1–8",
            ));
        }
        Ok(self)
    }
    pub fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }
}

pub(super) fn is_loopback(url: &reqwest::Url) -> bool {
    url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    })
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct SavedConfig {
    pub config: ProviderConfig,
    #[serde(default)]
    pub api_key: Secret,
}

impl SavedConfig {
    pub fn has_key(&self) -> bool {
        !self.api_key.0.is_empty()
    }
}

fn validate_key(key: &Secret) -> Result<(), AppError> {
    if key.0.len() > 4096 || key.0.chars().any(char::is_control) {
        return Err(error("Invalid AI key"));
    }
    Ok(())
}

fn decode(value: serde_json::Value) -> Result<SavedConfig, AppError> {
    let saved: SavedConfig =
        serde_json::from_value(value).map_err(|_| error("Invalid AI settings file"))?;
    saved.config.clone().validated()?;
    validate_key(&saved.api_key)?;
    Ok(saved)
}

pub(super) fn load(directory: &Path) -> Result<SavedConfig, AppError> {
    crate::config::with_settings_file(directory, |file, _| {
        file.ai
            .clone()
            .map_or_else(|| Ok(SavedConfig::default()), decode)
    })
    .map_err(|_| error("Cannot load AI settings"))
}

pub(super) fn save(
    directory: &Path,
    previous: &SavedConfig,
    config: ProviderConfig,
    key: Secret,
    clear: bool,
) -> Result<SavedConfig, AppError> {
    let config = config.validated()?;
    validate_key(&key)?;
    if clear && !key.0.is_empty() {
        return Err(error("Choose either a new key or clear key"));
    }
    if key.0.is_empty()
        && !clear
        && config.base_url != previous.config.base_url
        && previous.has_key()
    {
        return Err(error(
            "Base URL changed: enter a key for the new provider or explicitly clear the old key",
        ));
    }
    let saved = SavedConfig {
        config,
        api_key: if clear {
            Secret::default()
        } else if key.0.is_empty() {
            previous.api_key.clone()
        } else {
            key
        },
    };
    persist(directory, &saved)
        .map_err(|_| error("Cannot save AI settings; previous settings retained"))?;
    Ok(saved)
}

fn persist(directory: &Path, saved: &SavedConfig) -> Result<(), AppError> {
    crate::config::with_settings_file(directory, |file, path| {
        file.ai = Some(serde_json::to_value(saved).map_err(|_| error("Cannot save AI settings"))?);
        file.persist(path)
    })
}

#[cfg(test)]
mod storage_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::PermissionsExt};

    fn config() -> ProviderConfig {
        ProviderConfig {
            base_url: "https://example.test/v1".into(),
            model: "fake".into(),
            ..Default::default()
        }
    }

    #[test]
    fn config_keys_survive_restarts_updates_and_clear() {
        let directory = crate::config::tests::TestDir::new("ai-file-key");
        let saved = save(
            &directory.0,
            &SavedConfig::default(),
            config(),
            Secret("test-only-key".into()),
            false,
        )
        .unwrap();
        let path = directory.0.join("settings.json");
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["ai"]["api_key"], "test-only-key");
        assert!(!directory.0.join("ai.json").exists());
        assert!(json["ai"].get("credential").is_none());
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let loaded = load(&directory.0).unwrap();
        assert_eq!(loaded.api_key, saved.api_key);
        assert!(loaded.has_key());
        let mut changed = config();
        changed.model = "another-model".into();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let updated = save(&directory.0, &loaded, changed, Secret::default(), false).unwrap();
        assert_eq!(load(&directory.0).unwrap().api_key, saved.api_key);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let mut other = config();
        other.base_url = "https://other.test/v1".into();
        assert!(
            save(
                &directory.0,
                &updated,
                other.clone(),
                Secret::default(),
                false
            )
            .is_err()
        );
        assert_eq!(load(&directory.0).unwrap().config.model, "another-model");
        let replaced = save(
            &directory.0,
            &updated,
            other.clone(),
            Secret("replacement".into()),
            false,
        )
        .unwrap();
        assert_eq!(load(&directory.0).unwrap().api_key, replaced.api_key);
        let cleared = save(&directory.0, &replaced, other, Secret::default(), true).unwrap();
        assert!(!cleared.has_key());
        assert!(!load(&directory.0).unwrap().has_key());
    }

    #[test]
    fn failed_config_writes_leave_previous_file_and_no_key_temp() {
        let directory = crate::config::tests::TestDir::new("ai-write-failure");
        let saved = save(
            &directory.0,
            &SavedConfig::default(),
            config(),
            Secret("old-key".into()),
            false,
        )
        .unwrap();
        let path = directory.0.join("settings.json");
        let before = fs::read(&path).unwrap();
        fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o500)).unwrap();
        let result = save(
            &directory.0,
            &saved,
            config(),
            Secret("new-key".into()),
            false,
        );
        fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);
        assert!(!result.err().unwrap().message.contains("new-key"));
    }
    #[test]
    fn destination_is_explicit_and_secrets_cannot_enter_url() {
        for url in [
            "http://example.com/v1",
            "https://key@example.com/v1",
            "https://example.com/v1?key=x",
            "file:///tmp/key",
        ] {
            assert!(
                ProviderConfig {
                    base_url: url.into(),
                    model: "test".into(),
                    ..Default::default()
                }
                .validated()
                .is_err()
            );
        }
        for url in [
            "http://127.0.0.1:1234/v1",
            "http://[::1]:1234/v1",
            "https://example.com/api/v1/",
        ] {
            assert!(
                ProviderConfig {
                    base_url: url.into(),
                    model: "test".into(),
                    ..Default::default()
                }
                .validated()
                .is_ok()
            );
        }
        assert!(!format!("{:?}", Secret("very-secret".into())).contains("very-secret"));
    }
}
