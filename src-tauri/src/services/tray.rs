use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    AppHandle, Manager,
};

pub fn setup_tray(app: &AppHandle) -> anyhow::Result<()> {
    let toggle = MenuItemBuilder::with_id("toggle", "Toggle").build(app);
    let exit = MenuItemBuilder::with_id("exit", "Exit").build(app);

    let menu = MenuBuilder::new(app).items(&[&toggle, &exit]).build()?;

    let tray = TrayIconBuilder::with_id("my-tray")
        .icon(tauri::Icon::Raw(
            include_bytes!("../../icons/tray-logo.png").to_vec(),
        ))
        .icon_as_template(true)
        .menu(&menu)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "toggle" => {
                if let Some(window) = app.get_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "exit" => {
                let _ = app.exit(0);
            }
            _ => (),
        });

    #[cfg(not(target_os = "macos"))]
    let tray = tray.on_tray_icon_event(|tray, event| {
        use tauri::tray::ClickType;

        if event.click_type == ClickType::Left {
            let app = tray.app_handle();
            if let Some(window) = app.get_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
    });

    tray.build(app)?;

    Ok(())
}
