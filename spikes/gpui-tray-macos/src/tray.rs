use anyhow::{Context as _, Result};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuId, MenuItem},
};

use crate::command::AppCommand;

const TRAY_ICON: &[u8] = include_bytes!("../../../src-tauri/icons/tray-logo.png");

pub struct TrayService {
    _tray: TrayIcon,
}

impl TrayService {
    pub fn new(on_command: impl Fn(AppCommand) + Send + Sync + 'static) -> Result<Self> {
        let menu = Menu::new();
        let show = MenuItem::with_id(MenuId::new("show-main-window"), "Show Verge", true, None);
        let hide = MenuItem::with_id(MenuId::new("hide-main-window"), "Hide Verge", true, None);
        let quit = MenuItem::with_id(MenuId::new("quit"), "Quit", true, None);

        menu.append_items(&[&show, &hide, &quit])?;
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(command) = AppCommand::from_menu_id(event.id().as_ref()) {
                on_command(command);
            }
        }));

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Verge GPUI tray spike")
            .with_icon(load_icon()?)
            .build()
            .context("failed to build tray icon")?;

        Ok(Self { _tray: tray })
    }
}

fn load_icon() -> Result<Icon> {
    let image = image::load_from_memory(TRAY_ICON)
        .context("failed to decode tray icon")?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height).context("failed to create tray icon")
}
