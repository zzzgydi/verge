use super::{Secret, error};
use crate::domain::AppError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

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
        let local = url.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
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

#[derive(Default, Serialize, Deserialize)]
pub(super) struct SavedConfig {
    pub config: ProviderConfig,
    pub credential: Option<String>,
}

const SERVICE: &str = "com.zzzgydi.verge.ai";

pub(super) trait Credentials {
    fn set(&self, account: &str, key: &str) -> Result<(), AppError>;
    fn get(&self, account: &str) -> Result<Secret, AppError>;
    fn delete(&self, account: &str) -> Result<(), AppError>;
}

pub(super) struct Keychain;
impl Credentials for Keychain {
    fn set(&self, account: &str, key: &str) -> Result<(), AppError> {
        security_framework::passwords::set_generic_password(SERVICE, account, key.as_bytes())
            .map_err(|_| error("Unable to save AI key in Keychain"))
    }
    fn get(&self, account: &str) -> Result<Secret, AppError> {
        let bytes = security_framework::passwords::get_generic_password(SERVICE, account).map_err(
            |_| error("Unable to read AI key from Keychain; unlock Keychain or save the key again"),
        )?;
        String::from_utf8(bytes)
            .map(Secret)
            .map_err(|_| error("Invalid AI key encoding"))
    }
    fn delete(&self, account: &str) -> Result<(), AppError> {
        security_framework::passwords::delete_generic_password(SERVICE, account)
            .map_err(|_| error("Unable to remove previous AI key from Keychain"))
    }
}

pub(super) fn load(directory: &Path) -> Result<SavedConfig, AppError> {
    let path = directory.join("ai.json");
    if !path.exists() {
        return Ok(SavedConfig::default());
    }
    if fs::metadata(&path)
        .map_err(|_| error("Cannot read AI settings"))?
        .len()
        > 8192
    {
        return Err(error("AI settings file is too large"));
    }
    let bytes = fs::read(path).map_err(|_| error("Cannot read AI settings"))?;
    let saved: SavedConfig =
        serde_json::from_slice(&bytes).map_err(|_| error("Invalid AI settings file"))?;
    saved.config.clone().validated()?;
    Ok(saved)
}

pub(super) fn save(
    directory: &Path,
    previous: &SavedConfig,
    config: ProviderConfig,
    key: Secret,
    clear: bool,
    credentials: &impl Credentials,
) -> Result<SavedConfig, AppError> {
    let config = config.validated()?;
    if key.0.len() > 4096 || key.0.chars().any(char::is_control) {
        return Err(error("Invalid AI key"));
    }
    if clear && !key.0.is_empty() {
        return Err(error("Choose either a new key or clear key"));
    }
    let same_endpoint = config.base_url == previous.config.base_url;
    if !same_endpoint && key.0.is_empty() && !clear && previous.credential.is_some() {
        return Err(error(
            "Base URL changed: enter a key for the new provider or explicitly clear the old key",
        ));
    }
    let new_account = if key.0.is_empty() {
        None
    } else {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).map_err(|_| error("Cannot generate Keychain reference"))?;
        Some(format!(
            "{:x}-{:x}",
            Sha256::digest(directory.as_os_str().as_encoded_bytes()),
            Sha256::digest(random)
        ))
    };
    if let Some(account) = &new_account {
        credentials.set(account, &key.0)?;
    }
    let saved = SavedConfig {
        config,
        credential: new_account.clone().or_else(|| {
            if clear || !same_endpoint {
                None
            } else {
                previous.credential.clone()
            }
        }),
    };
    if persist(directory, &saved).is_err() {
        if let Some(account) = new_account {
            let _ = credentials.delete(&account);
        }
        return Err(error("Cannot save AI settings; previous settings retained"));
    }
    if let Some(old) = &previous.credential
        && saved.credential.as_ref() != Some(old)
    {
        let _ = credentials.delete(old);
    }
    Ok(saved)
}

fn persist(directory: &Path, saved: &SavedConfig) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::create_dir_all(directory)?;
    let path: PathBuf = directory.join("ai.json.tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)?;
    file.write_all(&serde_json::to_vec_pretty(saved)?)?;
    file.sync_all()?;
    fs::rename(path, directory.join("ai.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct FakeKeys(std::cell::RefCell<std::collections::HashMap<String, String>>);
    impl Credentials for FakeKeys {
        fn set(&self, account: &str, key: &str) -> Result<(), AppError> {
            self.0.borrow_mut().insert(account.into(), key.into());
            Ok(())
        }
        fn get(&self, account: &str) -> Result<Secret, AppError> {
            self.0
                .borrow()
                .get(account)
                .cloned()
                .map(Secret)
                .ok_or_else(|| error("missing"))
        }
        fn delete(&self, account: &str) -> Result<(), AppError> {
            self.0.borrow_mut().remove(account);
            Ok(())
        }
    }
    #[test]
    fn settings_keep_keys_out_of_files_and_roll_back_failed_saves() {
        let directory =
            std::env::temp_dir().join(format!("verge-ai-settings-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let keys = FakeKeys::default();
        let config = ProviderConfig {
            base_url: "https://example.test/v1".into(),
            model: "fake".into(),
            ..Default::default()
        };
        let saved = save(
            &directory,
            &SavedConfig::default(),
            config.clone(),
            Secret("test-only-key".into()),
            false,
            &keys,
        )
        .unwrap();
        assert!(
            !fs::read_to_string(directory.join("ai.json"))
                .unwrap()
                .contains("test-only-key")
        );
        assert_eq!(
            keys.get(saved.credential.as_ref().unwrap()).unwrap().0,
            "test-only-key"
        );
        let mut changed = config.clone();
        changed.base_url = "https://other.test/v1".into();
        assert!(save(&directory, &saved, changed, Secret::default(), false, &keys).is_err());
        fs::create_dir(directory.join("ai.json.tmp")).unwrap();
        assert!(
            save(
                &directory,
                &saved,
                config.clone(),
                Secret("replacement-key".into()),
                false,
                &keys
            )
            .is_err()
        );
        assert_eq!(keys.0.borrow().len(), 1);
        assert_eq!(load(&directory).unwrap().credential, saved.credential);
        fs::remove_dir(directory.join("ai.json.tmp")).unwrap();
        let cleared = save(&directory, &saved, config, Secret::default(), true, &keys).unwrap();
        assert!(cleared.credential.is_none());
        assert!(keys.0.borrow().is_empty());
        fs::remove_dir_all(directory).unwrap();
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
