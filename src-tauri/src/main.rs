// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use services::{menu, tray};
use tauri::Manager;
use tauri_plugin_log::{Target, TargetKind};
use utils::dirs::log_dir;

mod core;
mod entities;
mod services;
mod transform;
mod utils;

#[tauri::command]
fn greet(name: &str) -> String {
    log::info!("Hello, {}!", name);
    format!("Hello, {}! You've been greeted from Rust!", name)
}

fn main() {
    let app = tauri::Builder::default()
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
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    #[cfg(not(target_os = "macos"))]
                    {
                        event.window().hide().unwrap();
                    }

                    #[cfg(target_os = "macos")]
                    {
                        let _ = tauri::AppHandle::hide(&event.window().app_handle());
                    }
                    api.prevent_close();
                }
                _ => {}
            }
            // TODO
        })
        .setup(|app| {
            let _ = menu::setup_menu(app.app_handle());

            let _ = tray::setup_tray(app.app_handle());

            app.path().resource_dir();
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    app.run(|app_handle, e| match e {
        tauri::RunEvent::ExitRequested { api, .. } => {
            api.prevent_exit();
        }
        tauri::RunEvent::Exit => {
            println!("exiting");
        }
        tauri::RunEvent::MainEventsCleared => {}

        a @ _ => {
            println!("{:?}", a);
        }
    });
}
