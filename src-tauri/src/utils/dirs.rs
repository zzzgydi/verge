use anyhow::Result;
use dirs_next::home_dir;
use std::path::PathBuf;
use tauri::utils::platform::current_exe;

use crate::transform::from_clash::OptionToResult;

#[cfg(not(feature = "verge-dev"))]
static APP_DIR: &str = "verge";
#[cfg(feature = "verge-dev")]
static APP_DIR: &str = "verge-dev";

pub fn app_dir() -> Result<PathBuf> {
    Ok(home_dir()
        .okr("failed to get the app home dir")?
        .join(".config")
        .join(APP_DIR))
}

pub fn log_dir() -> Result<PathBuf> {
    Ok(app_dir()?.join("logs"))
}

pub fn bin_dir() -> Result<PathBuf> {
    let cur = current_exe()?;
    Ok(cur
        .parent()
        .okr("failed to get the bin dir")?
        .to_path_buf()
        .join("bin"))
}
