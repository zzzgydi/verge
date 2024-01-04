// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use services::{menu, tray};
use tauri::Manager;
use tauri_plugin_log::{Target, TargetKind};
use utils::log_dir;

mod cmds;
mod core;
mod entities;
mod services;
mod transform;
mod utils;

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_window::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
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
        .invoke_handler(tauri::generate_handler![
            cmds::list_profiles,
            cmds::import_profile,
        ])
        .on_window_event(|event| {
            match event.event() {
                // tauri::WindowEvent::Focused(focused) => {
                //     log::info!("Window focused: {}", focused);
                // }
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
            utils::resolve_setup(app)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    app.run(|app_handle, e| match e {
        tauri::RunEvent::ExitRequested { api, .. } => {
            api.prevent_exit();
        }
        tauri::RunEvent::WindowEvent { event, .. } => match event {
            tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Resized(_) => {
                use tauri_plugin_window_state::{AppHandleExt, StateFlags};
                let res = app_handle.save_window_state(StateFlags::all());
                log::info!("restore state: {:?}", res);
            }
            _ => {}
        },
        _ => {}
    });
}
