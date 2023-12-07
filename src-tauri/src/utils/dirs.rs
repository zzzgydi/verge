use anyhow::Result;
use dirs_next::home_dir;
use std::path::PathBuf;

#[cfg(not(feature = "verge-dev"))]
static APP_DIR: &str = "verge";
#[cfg(feature = "verge-dev")]
static APP_DIR: &str = "verge-dev";

pub fn app_dir() -> Result<PathBuf> {
    Ok(home_dir()
        .ok_or(anyhow::anyhow!("failed to get the app home dir"))?
        .join(".config")
        .join(APP_DIR))
}

pub fn log_dir() -> Result<PathBuf> {
    Ok(app_dir()?.join("logs"))
}
