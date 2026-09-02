use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem},
};
use crate::domain::{AppError, ErrorCode};

const TRAY_ICON: &[u8] = include_bytes!("../../../../assets/icons/tray-logo.png");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrayCommand {
    ShowMainWindow,
    HideMainWindow,
    ToggleSystemProxy,
    Quit,
}

impl TrayCommand {
    fn from_menu_id(id: &str) -> Option<Self> {
        match id {
            "show-main-window" => Some(Self::ShowMainWindow),
            "hide-main-window" => Some(Self::HideMainWindow),
            "toggle-system-proxy" => Some(Self::ToggleSystemProxy),
            "quit" => Some(Self::Quit),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraySnapshot {
    pub system_proxy_enabled: bool,
    pub upload_bytes_per_second: u64,
    pub download_bytes_per_second: u64,
    /// 界面语言（"zh-CN" / "en"）；守护进程从应用设置带入，语言变化时菜单文案随之更新。
    pub language: String,
}

impl Default for TraySnapshot {
    fn default() -> Self {
        Self {
            system_proxy_enabled: false,
            upload_bytes_per_second: 0,
            download_bytes_per_second: 0,
            // 与 ApplicationSettings 的默认语言一致；未知值也走英文。
            language: "en".into(),
        }
    }
}

/// 托盘菜单文案（双语查表，与 GUI 的 i18n 模块同风格，跨 crate 不共享）。
struct TrayLabels {
    show: &'static str,
    hide: &'static str,
    system_proxy: &'static str,
    quit: &'static str,
}

fn tray_labels(language: &str) -> TrayLabels {
    match language {
        "zh-CN" => TrayLabels {
            show: "显示 Verge",
            hide: "隐藏 Verge",
            system_proxy: "系统代理",
            quit: "退出",
        },
        _ => TrayLabels {
            show: "Show Verge",
            hide: "Hide Verge",
            system_proxy: "System Proxy",
            quit: "Quit",
        },
    }
}

pub struct TrayService {
    tray: TrayIcon,
    show: MenuItem,
    hide: MenuItem,
    system_proxy: CheckMenuItem,
    speed: MenuItem,
    quit: MenuItem,
    language: std::sync::Mutex<String>,
}

impl TrayService {
    pub fn new(on_command: impl Fn(TrayCommand) + Send + Sync + 'static) -> Result<Self, AppError> {
        // 初始用默认语言建菜单；守护进程启动后会立刻推一次带实际语言的快照。
        let labels = tray_labels("en");
        let menu = Menu::new();
        let show = MenuItem::with_id(MenuId::new("show-main-window"), labels.show, true, None);
        let hide = MenuItem::with_id(MenuId::new("hide-main-window"), labels.hide, true, None);
        let system_proxy = CheckMenuItem::with_id(
            MenuId::new("toggle-system-proxy"),
            labels.system_proxy,
            true,
            false,
            None,
        );
        let speed = MenuItem::new("↑ 0 B/s   ↓ 0 B/s", false, None);
        let quit = MenuItem::with_id(MenuId::new("quit"), labels.quit, true, None);
        menu.append_items(&[&show, &hide, &system_proxy, &speed, &quit])
            .map_err(tray_error)?;
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(command) = TrayCommand::from_menu_id(event.id().as_ref()) {
                on_command(command);
            }
        }));
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Verge")
            .with_icon(load_icon()?)
            .build()
            .map_err(tray_error)?;
        Ok(Self {
            tray,
            show,
            hide,
            system_proxy,
            speed,
            quit,
            language: std::sync::Mutex::new("en".into()),
        })
    }

    pub fn update(&self, snapshot: &TraySnapshot) -> Result<(), AppError> {
        self.system_proxy.set_checked(snapshot.system_proxy_enabled);
        let speed = format!(
            "↑ {}/s   ↓ {}/s",
            format_bytes(snapshot.upload_bytes_per_second),
            format_bytes(snapshot.download_bytes_per_second)
        );
        self.speed.set_text(&speed);
        // 语言变化时重贴菜单文案（菜单项按 id 复用，无需重建菜单）。
        let mut language = self
            .language
            .lock()
            .expect("tray language mutex poisoned");
        if *language != snapshot.language {
            *language = snapshot.language.clone();
            let labels = tray_labels(&snapshot.language);
            self.show.set_text(labels.show);
            self.hide.set_text(labels.hide);
            self.system_proxy.set_text(labels.system_proxy);
            self.quit.set_text(labels.quit);
        }
        drop(language);
        self.tray
            .set_tooltip(Some(format!("Verge · {speed}")))
            .map_err(tray_error)
    }
}

fn load_icon() -> Result<Icon, AppError> {
    let image = image::load_from_memory(TRAY_ICON)
        .map_err(tray_error)?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height).map_err(tray_error)
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn tray_error(error: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::PlatformFailed, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_menu_ids_and_formats_speed_without_gui_state() {
        assert_eq!(
            TrayCommand::from_menu_id("toggle-system-proxy"),
            Some(TrayCommand::ToggleSystemProxy)
        );
        assert_eq!(TrayCommand::from_menu_id("unknown"), None);
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(2_048), "2.0 KiB");
        assert_eq!(format_bytes(2 * 1024 * 1024), "2.0 MiB");
    }

    #[test]
    fn tray_labels_follow_language() {
        let zh = tray_labels("zh-CN");
        assert_eq!(zh.show, "显示 Verge");
        assert_eq!(zh.system_proxy, "系统代理");
        assert_eq!(zh.quit, "退出");
        let en = tray_labels("en");
        assert_eq!(en.show, "Show Verge");
        // 未知语言码回退英文。
        assert_eq!(tray_labels("fr").show, "Show Verge");
        assert_eq!(TraySnapshot::default().language, "en");
    }
}
