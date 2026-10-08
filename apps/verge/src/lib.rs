pub mod ai;
mod appearance;
pub mod application;
mod assets;
pub mod config;
pub mod daemon;
pub mod domain;
mod format;
mod gui;
mod i18n;
pub mod identity;
pub mod ipc;
pub mod mihomo;
mod pages;
pub mod platform;
pub mod script;
pub mod ui;
mod view;

pub fn run() {
    if std::env::args().nth(1).as_deref() == Some("--core-watch") {
        if let Err(error) = mihomo::core_watch_main() {
            eprintln!("Core lease ended: {error}");
            std::process::exit(1);
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("--script-worker") {
        script::worker_main();
        return;
    }
    gui::run();
}
