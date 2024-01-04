use crate::{menu, services::core::Core, tray, utils};
use anyhow::{bail, Result};
use std::fs;
use tauri::Manager;

pub fn resolve_setup(app: &mut tauri::App) -> Result<()> {
    let _ = menu::setup_menu(app.app_handle());

    let _ = tray::setup_tray(app.app_handle());

    // if let Err(e) = resolve_bin(app) {
    //     log::error!("failed to resolve bin: {e}");
    // }

    Ok(())
}

fn resolve_bin(app: &mut tauri::App) -> Result<()> {
    let res_dir = app.path().resource_dir()?.join("resources");
    let res_bin = res_dir.join("bin").join("sing-box");
    if !res_bin.exists() {
        bail!("sing-box binary not found in resource dir");
    }

    let target = utils::bin_dir()?;
    let bin = target.join("sing-box");

    let _ = fs::create_dir_all(&target);

    if !bin.exists() {
        fs::copy(&res_bin, &bin)?;
    }

    tauri::async_runtime::block_on(async {
        if let Err(e) = Core::global().run_core().await {
            log::error!("failed to run core: {e}");
        }
    });

    Ok(())
}
