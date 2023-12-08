// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use services::menu;
use tauri::Manager;
use tauri_plugin_log::{Target, TargetKind};
use utils::dirs::log_dir;

mod core;
mod entities;
mod services;
mod utils;

#[tauri::command]
fn greet(name: &str) -> String {
    log::info!("Hello, {}!", name);
    format!("Hello, {}! You've been greeted from Rust!", name)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_window::init())
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            println!("{}, {argv:?}, {cwd}", app.package_info().name);
            // TODO
            // macOS TODO
        }))
        .plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .targets([
                    Target::new(TargetKind::Folder {
                        path: log_dir().expect("failed to get log dir"),
                        file_name: None,
                    }),
                    Target::new(TargetKind::Stdout),
                ])
                .build(),
        )
        .invoke_handler(tauri::generate_handler![greet])
        .on_window_event(|event| {
            match event.event() {
                tauri::WindowEvent::Focused(focused) => {
                    log::info!("Window focused: {}", focused);
                }
                _ => {}
            }
            // TODO
        })
        .setup(|app| {
            let _ = menu::setup_menu(app.app_handle());

            let tray = tauri::tray::TrayIconBuilder::with_id("my-tray")
                .icon(tauri::Icon::Raw(
                    include_bytes!("../icons/tray-logo.png").to_vec(),
                ))
                .icon_as_template(true)
                .build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
