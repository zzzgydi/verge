//! Build identity is independent of optimization and inherited by every process.
use crate::domain::{AppError, ErrorCode};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppChannel {
    Stable,
    Dev,
}

impl AppChannel {
    pub fn current() -> Self {
        match env!("VERGE_BUILD_CHANNEL") {
            "dev" => Self::Dev,
            _ => Self::Stable,
        }
    }
    pub fn is_dev(self) -> bool {
        self == Self::Dev
    }
    pub fn name(self) -> &'static str {
        if self.is_dev() { "Verge Dev" } else { "Verge" }
    }
    pub fn id(self) -> &'static str {
        if self.is_dev() { "dev" } else { "stable" }
    }
    pub fn ai_service(self) -> &'static str {
        if self.is_dev() {
            "com.zzzgydi.verge.dev.ai"
        } else {
            "com.zzzgydi.verge.ai"
        }
    }
    pub fn network_service(self) -> &'static str {
        if self.is_dev() {
            "com.zzzgydi.verge.dev.network"
        } else {
            "com.zzzgydi.verge.network"
        }
    }
    pub fn data_directory(
        self,
        home: &Path,
        override_path: Option<&Path>,
    ) -> Result<PathBuf, AppError> {
        let support = home.join("Library/Application Support");
        let path = resolve_path(override_path.unwrap_or(&support.join(self.name())))?;
        if self.is_dev() {
            let stable = resolve_path(&support.join("Verge"))?;
            if path.starts_with(&stable) || stable.starts_with(&path) {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "Verge Dev cannot use the production data directory or its parent/children",
                ));
            }
        }
        if path.join("control/mihomo.sock").as_os_str().len() >= 104 {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Verge data directory is too long for Unix sockets",
            ));
        }
        Ok(path)
    }
}

pub fn dev_restriction() -> AppError {
    AppError::new(
        ErrorCode::PermissionDenied,
        "Verge Dev keeps system proxy, TUN, helper, login items and app updates under the installed app's control",
    )
}

// Resolve existing ancestors before creating anything, including symlinks and .. .
fn resolve_path(path: &Path) -> Result<PathBuf, AppError> {
    if !path.is_absolute() {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "VERGE_DATA_DIR must be an absolute path",
        ));
    }
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            other => resolved.push(other.as_os_str()),
        }
        match std::fs::symlink_metadata(&resolved) {
            Ok(_) => {
                resolved = resolved
                    .canonicalize()
                    .map_err(|e| AppError::new(ErrorCode::StorageFailed, e.to_string()))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(AppError::new(ErrorCode::StorageFailed, e.to_string())),
        }
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn channels_have_separate_data_and_credentials() {
        let home = Path::new("/tmp/verge-channel-home");
        let stable = AppChannel::Stable.data_directory(home, None).unwrap();
        let dev = AppChannel::Dev.data_directory(home, None).unwrap();
        assert_ne!(stable, dev);
        assert_ne!(
            AppChannel::Stable.ai_service(),
            AppChannel::Dev.ai_service()
        );
        assert_ne!(
            AppChannel::Stable.network_service(),
            AppChannel::Dev.network_service()
        );
        for path in [&stable, &stable.join("child"), stable.parent().unwrap()] {
            assert!(AppChannel::Dev.data_directory(home, Some(path)).is_err());
        }
        assert!(
            AppChannel::Dev
                .data_directory(home, Some(Path::new("relative")))
                .is_err()
        );
    }
    #[test]
    fn dev_rejects_symlinks_into_production_before_writing() {
        let root = std::env::temp_dir().join(format!("verge-identity-{}", std::process::id()));
        let stable = root.join("Library/Application Support/Verge");
        std::fs::create_dir_all(&stable).unwrap();
        let alias = root.join("alias");
        std::os::unix::fs::symlink(&stable, &alias).unwrap();
        assert!(AppChannel::Dev.data_directory(&root, Some(&alias)).is_err());
        assert!(
            AppChannel::Dev
                .data_directory(&root, Some(&alias.join("new")))
                .is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
