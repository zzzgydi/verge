use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use crate::domain::{AppError, ErrorCode};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct SidecarManifest {
    pub schema_version: u32,
    pub version: String,
    pub release_url: String,
    pub targets: std::collections::BTreeMap<String, SidecarTarget>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactInstall {
    pub active: PathBuf,
    pub previous: Option<PathBuf>,
}

pub fn install_verified_artifact(
    candidate: impl AsRef<Path>,
    active: impl AsRef<Path>,
    expected_sha256: &str,
) -> Result<ArtifactInstall, AppError> {
    validate_sha256(expected_sha256)?;
    let candidate = candidate.as_ref();
    let active = active.as_ref();
    let bytes = fs::read(candidate).map_err(sidecar_error)?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual != expected_sha256.to_ascii_lowercase() {
        return Err(AppError::new(
            ErrorCode::ValidationFailed,
            format!("artifact digest mismatch: expected {expected_sha256}, got {actual}"),
        ));
    }
    if let Some(parent) = active.parent() {
        fs::create_dir_all(parent).map_err(sidecar_error)?;
    }
    let staged = active.with_extension("staged");
    let previous = active.with_extension("previous");
    fs::write(&staged, bytes).map_err(sidecar_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(candidate)
            .map_err(sidecar_error)?
            .permissions()
            .mode();
        fs::set_permissions(&staged, fs::Permissions::from_mode(mode)).map_err(sidecar_error)?;
    }
    if previous.exists() {
        fs::remove_file(&previous).map_err(sidecar_error)?;
    }
    let had_active = active.exists();
    if had_active {
        fs::rename(active, &previous).map_err(sidecar_error)?;
    }
    if let Err(error) = fs::rename(&staged, active) {
        if had_active {
            let _ = fs::rename(&previous, active);
        }
        return Err(sidecar_error(error));
    }
    Ok(ArtifactInstall {
        active: active.to_owned(),
        previous: had_active.then_some(previous),
    })
}

impl ArtifactInstall {
    pub fn commit(self) -> Result<(), AppError> {
        if let Some(previous) = self.previous {
            match fs::remove_file(previous) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(sidecar_error(error)),
            }
        }
        Ok(())
    }

    pub fn rollback(self) -> Result<(), AppError> {
        if let Some(previous) = self.previous {
            fs::remove_file(&self.active).map_err(sidecar_error)?;
            fs::rename(previous, self.active).map_err(sidecar_error)
        } else {
            fs::remove_file(self.active).map_err(sidecar_error)
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct SidecarTarget {
    pub asset: String,
    pub download_url: String,
    pub archive_sha256: String,
    pub executable_sha256: String,
}

impl SidecarManifest {
    pub fn load(path: &Path) -> Result<Self, AppError> {
        let bytes = fs::read(path).map_err(sidecar_error)?;
        let manifest: Self = serde_json::from_slice(&bytes).map_err(sidecar_error)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn target(&self, target: &str) -> Result<&SidecarTarget, AppError> {
        self.targets.get(target).ok_or_else(|| {
            AppError::new(
                ErrorCode::CoreUnavailable,
                format!("Mihomo manifest has no asset for target {target}"),
            )
        })
    }

    fn validate(&self) -> Result<(), AppError> {
        if self.schema_version != 1 {
            return Err(AppError::new(
                ErrorCode::CoreUnavailable,
                format!(
                    "unsupported Mihomo manifest schema version: {}",
                    self.schema_version
                ),
            ));
        }
        if self.version.trim().is_empty() || self.targets.is_empty() {
            return Err(AppError::new(
                ErrorCode::CoreUnavailable,
                "Mihomo manifest has no version or targets",
            ));
        }
        for target in self.targets.values() {
            validate_sha256(&target.archive_sha256)?;
            validate_sha256(&target.executable_sha256)?;
            if !target
                .download_url
                .starts_with("https://github.com/MetaCubeX/mihomo/releases/")
            {
                return Err(AppError::new(
                    ErrorCode::CoreUnavailable,
                    "Mihomo asset URL is not an official GitHub release URL",
                ));
            }
        }
        Ok(())
    }
}

impl SidecarTarget {
    pub fn verify_archive(&self, path: &Path) -> Result<(), AppError> {
        verify_sha256(path, &self.archive_sha256)
    }

    pub fn verify_executable(&self, path: &Path) -> Result<(), AppError> {
        verify_sha256(path, &self.executable_sha256)
    }
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), AppError> {
    let bytes = fs::read(path).map_err(|error| {
        AppError::new(
            ErrorCode::CoreUnavailable,
            format!("cannot read Mihomo file {}: {error}", path.display()),
        )
    })?;
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual == expected {
        Ok(())
    } else {
        Err(AppError::new(
            ErrorCode::CoreUnavailable,
            format!(
                "Mihomo SHA-256 mismatch for {}: expected {expected}, got {actual}",
                path.display()
            ),
        ))
    }
}

fn validate_sha256(value: &str) -> Result<(), AppError> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(AppError::new(
            ErrorCode::CoreUnavailable,
            "Mihomo manifest contains an invalid SHA-256",
        ))
    }
}

fn sidecar_error(error: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::CoreUnavailable, error.to_string())
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn verifies_file_digest_and_rejects_tampering() {
        let path = std::env::temp_dir().join(format!(
            "verge-sidecar-hash-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, b"mihomo").unwrap();
        let digest = format!("{:x}", Sha256::digest(b"mihomo"));
        verify_sha256(&path, &digest).unwrap();
        let error = verify_sha256(&path, &"0".repeat(64)).unwrap_err();
        assert!(error.message.contains("mismatch"));
        let _ = fs::remove_file(path);
    }

    fn test_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "verge-artifact-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn installs_verified_artifact_and_commits_previous_version() {
        let dir = test_dir("commit");
        let candidate = dir.join("candidate");
        let active = dir.join("active");
        fs::write(&candidate, b"new artifact").unwrap();
        fs::write(&active, b"old artifact").unwrap();
        let digest = format!("{:x}", Sha256::digest(b"new artifact"));

        let install = install_verified_artifact(&candidate, &active, &digest).unwrap();
        let previous = install.previous.clone().unwrap();
        assert_eq!(fs::read(&active).unwrap(), b"new artifact");
        assert_eq!(fs::read(&previous).unwrap(), b"old artifact");
        assert_eq!(fs::read(&candidate).unwrap(), b"new artifact");

        install.commit().unwrap();
        assert!(!previous.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rolls_back_to_previous_artifact() {
        let dir = test_dir("rollback");
        let candidate = dir.join("candidate");
        let active = dir.join("active");
        fs::write(&candidate, b"new artifact").unwrap();
        fs::write(&active, b"old artifact").unwrap();
        let digest = format!("{:x}", Sha256::digest(b"new artifact"));

        install_verified_artifact(&candidate, &active, &digest)
            .unwrap()
            .rollback()
            .unwrap();

        assert_eq!(fs::read(&active).unwrap(), b"old artifact");
        assert!(!active.with_extension("previous").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn bad_digest_leaves_active_artifact_untouched() {
        let dir = test_dir("bad-digest");
        let candidate = dir.join("candidate");
        let active = dir.join("active");
        fs::write(&candidate, b"new artifact").unwrap();
        fs::write(&active, b"old artifact").unwrap();

        let error = install_verified_artifact(&candidate, &active, &"0".repeat(64)).unwrap_err();

        assert_eq!(error.code, ErrorCode::ValidationFailed);
        assert_eq!(fs::read(&active).unwrap(), b"old artifact");
        assert!(!active.with_extension("staged").exists());
        assert!(!active.with_extension("previous").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rollback_removes_a_first_install() {
        let dir = test_dir("first-install");
        let candidate = dir.join("candidate");
        let active = dir.join("active");
        fs::write(&candidate, b"new artifact").unwrap();
        let digest = format!("{:x}", Sha256::digest(b"new artifact"));

        install_verified_artifact(&candidate, &active, &digest)
            .unwrap()
            .rollback()
            .unwrap();

        assert!(!active.exists());
        let _ = fs::remove_dir_all(dir);
    }
}
