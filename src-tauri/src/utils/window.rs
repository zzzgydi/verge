use anyhow::Result;
use tauri::{AppHandle, Manager};
use tauri_plugin_window_state::{StateFlags, WindowExt};

/// create main window
pub fn create_window(app_handle: &AppHandle) -> Result<()> {
    macro_rules! trace_err {
        ($result: expr, $err_str: expr) => {
            if let Err(err) = $result {
                log::trace!("{}, err {}", $err_str, err);
            }
        };
    }

    if let Some(window) = app_handle.get_window("main") {
        trace_err!(window.unminimize(), "set win unminimize");
        trace_err!(window.show(), "set win visible");
        trace_err!(window.set_focus(), "set win focus");
        return Ok(());
    }

    let builder = tauri::window::WindowBuilder::new(
        app_handle,
        "main".to_string(),
        tauri::WindowUrl::App("index.html".into()),
    )
    .title("Verge")
    .fullscreen(false)
    .min_inner_size(600.0, 520.0);

    #[cfg(target_os = "windows")]
    let window = builder.decorations(false).transparent(true).build();

    #[cfg(target_os = "macos")]
    let window = builder
        .decorations(true)
        .hidden_title(true)
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .build()?;

    #[cfg(target_os = "linux")]
    let window = builder.decorations(true).transparent(false).build()?;

    window.restore_state(StateFlags::all())?;

    Ok(())
}
