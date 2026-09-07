mod appearance;
pub mod application;
pub mod config;
pub mod daemon;
pub mod domain;
mod format;
mod gui;
mod i18n;
pub mod ipc;
pub mod mihomo;
mod pages;
pub mod platform;
pub mod ui;
mod view;

pub fn run() {
    gui::run();
}
