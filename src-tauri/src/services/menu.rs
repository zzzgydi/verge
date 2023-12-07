use tauri::{
    menu::{AboutMetadata, MenuBuilder, PredefinedMenuItem, SubmenuBuilder},
    AppHandle,
};

pub fn setup_menu(handle: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let pkg_info = handle.package_info();
    let metadata = AboutMetadata {
        name: Some("Verge".into()),
        version: Some(pkg_info.version.to_string()),
        ..AboutMetadata::default()
    };
    let app_menu = SubmenuBuilder::new(handle, "Verge")
        .close_window()
        .build()?;

    let native_menu = SubmenuBuilder::new(handle, "Edit")
        .items(&[
            &PredefinedMenuItem::about(handle, None, Some(metadata)),
            &PredefinedMenuItem::undo(handle, None),
            &PredefinedMenuItem::redo(handle, None),
            &PredefinedMenuItem::cut(handle, None),
            &PredefinedMenuItem::copy(handle, None),
            &PredefinedMenuItem::paste(handle, None),
            &PredefinedMenuItem::select_all(handle, None),
            &PredefinedMenuItem::fullscreen(handle, None),
            &PredefinedMenuItem::minimize(handle, None),
            &PredefinedMenuItem::maximize(handle, None),
        ])
        .build()?;

    let menus = MenuBuilder::new(handle).item(&app_menu).item(&native_menu);

    handle.set_menu(menus.build()?)?;
    handle.on_menu_event(move |handle, event| {
        println!("event id: {:?}", event.id);
    });

    Ok(())
}
