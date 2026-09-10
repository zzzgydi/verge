use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::domain::{AppError, ErrorCode, ProfileId, ProxyGroup, RunMode};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{
        CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
        accelerator::{Accelerator, Code, Modifiers},
    },
};

const TRAY_ICON: &[u8] = include_bytes!("../../../../assets/icons/tray-logo.png");

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TrayCommand {
    ShowMainWindow,
    ToggleSystemProxy,
    SetMode(RunMode),
    SelectProfile(ProfileId),
    SelectProxy { group: String, proxy: String },
    UpdateCurrentProfile,
    OpenDirectory(TrayDirectory),
    RestartCore,
    RestartApplication,
    Quit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrayDirectory {
    Data,
    Profiles,
    Logs,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TrayMenuState {
    pub language: String,
    pub system_proxy_enabled: bool,
    pub can_enable_system_proxy: bool,
    pub mode: Option<RunMode>,
    pub profiles: Vec<(ProfileId, String)>,
    pub selected_profile: Option<ProfileId>,
    pub can_update_profile: bool,
    pub can_restart_application: bool,
    pub groups: Vec<ProxyGroup>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TraySnapshot {
    pub menu: Arc<TrayMenuState>,
    pub upload_bytes_per_second: u64,
    pub download_bytes_per_second: u64,
}

/// Native menu description is independent of AppKit and contains only supported actions.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Entry {
    Item {
        label: String,
        command: Option<TrayCommand>,
        checked: Option<bool>,
        enabled: bool,
    },
    Submenu(String, Vec<Entry>),
    Separator,
}

impl Entry {
    fn action(label: impl Into<String>, command: TrayCommand, enabled: bool) -> Self {
        Self::Item {
            label: label.into(),
            command: Some(command),
            checked: None,
            enabled,
        }
    }
    fn check(label: impl Into<String>, command: TrayCommand, checked: bool, enabled: bool) -> Self {
        Self::Item {
            label: label.into(),
            command: Some(command),
            checked: Some(checked),
            enabled,
        }
    }
}

fn menu_entries(state: &TrayMenuState) -> Vec<Entry> {
    let tr = |zh: &'static str, en: &'static str| if state.language == "zh-CN" { zh } else { en };
    let mode_label = |mode| match mode {
        Some(RunMode::Rule) => tr("规则", "Rule"),
        Some(RunMode::Global) => tr("全局", "Global"),
        Some(RunMode::Direct) => tr("直连", "Direct"),
        None => tr("内核未运行", "Core offline"),
    };
    let profiles = state
        .profiles
        .iter()
        .map(|(id, name)| {
            Entry::check(
                name,
                TrayCommand::SelectProfile(id.clone()),
                state.selected_profile.as_ref() == Some(id),
                true,
            )
        })
        .collect();
    let group_items = |group: &ProxyGroup| {
        group
            .members
            .iter()
            .map(|proxy| {
                Entry::check(
                    proxy,
                    TrayCommand::SelectProxy {
                        group: group.name.clone(),
                        proxy: proxy.clone(),
                    },
                    group.selected.as_ref() == Some(proxy),
                    matches!(group.kind.as_str(), "Selector" | "URLTest" | "Fallback"),
                )
            })
            .collect::<Vec<_>>()
    };
    let proxies = match state.mode {
        Some(RunMode::Global) => state
            .groups
            .iter()
            .find(|g| g.name == "GLOBAL")
            .map(group_items)
            .unwrap_or_default(),
        Some(RunMode::Rule) => state
            .groups
            .iter()
            .filter(|g| g.name != "GLOBAL")
            .map(|group| {
                let title = group.selected.as_ref().map_or_else(
                    || group.name.clone(),
                    |selected| format!("{} · {selected}", group.name),
                );
                Entry::Submenu(title, group_items(group))
            })
            .collect(),
        _ => Vec::new(),
    };
    vec![
        Entry::action(
            tr("显示 Verge", "Show Verge"),
            TrayCommand::ShowMainWindow,
            true,
        ),
        Entry::Separator,
        Entry::Submenu(
            format!(
                "{}（{}）",
                tr("出站模式", "Outbound mode"),
                mode_label(state.mode)
            ),
            [RunMode::Rule, RunMode::Global, RunMode::Direct]
                .into_iter()
                .map(|mode| {
                    Entry::check(
                        mode_label(Some(mode)),
                        TrayCommand::SetMode(mode),
                        state.mode == Some(mode),
                        state.mode.is_some(),
                    )
                })
                .collect(),
        ),
        Entry::Separator,
        Entry::Submenu(tr("配置", "Profiles").into(), profiles),
        Entry::Submenu(tr("代理", "Proxies").into(), proxies),
        Entry::Separator,
        Entry::check(
            tr("系统代理", "System Proxy"),
            TrayCommand::ToggleSystemProxy,
            state.system_proxy_enabled,
            state.system_proxy_enabled || state.can_enable_system_proxy,
        ),
        Entry::Separator,
        Entry::Submenu(
            tr("打开目录", "Open Directory").into(),
            vec![
                Entry::action(
                    tr("数据目录", "Data"),
                    TrayCommand::OpenDirectory(TrayDirectory::Data),
                    true,
                ),
                Entry::action(
                    tr("配置目录", "Profiles"),
                    TrayCommand::OpenDirectory(TrayDirectory::Profiles),
                    true,
                ),
                Entry::action(
                    tr("日志目录", "Logs"),
                    TrayCommand::OpenDirectory(TrayDirectory::Logs),
                    true,
                ),
            ],
        ),
        Entry::Submenu(
            tr("更多", "More").into(),
            vec![
                Entry::action(
                    tr("更新当前订阅", "Update Current Subscription"),
                    TrayCommand::UpdateCurrentProfile,
                    state.can_update_profile,
                ),
                Entry::action(
                    tr("重启内核", "Restart Core"),
                    TrayCommand::RestartCore,
                    state.selected_profile.is_some(),
                ),
                Entry::action(
                    tr("重启 Verge", "Restart Verge"),
                    TrayCommand::RestartApplication,
                    state.can_restart_application,
                ),
                Entry::Separator,
                Entry::Item {
                    label: format!("Verge {}", env!("CARGO_PKG_VERSION")),
                    command: None,
                    checked: None,
                    enabled: false,
                },
            ],
        ),
        Entry::Separator,
        Entry::action(tr("退出", "Quit"), TrayCommand::Quit, true),
    ]
}

fn native_item(
    entry: &Entry,
    commands: &mut HashMap<String, TrayCommand>,
    checks: &mut Vec<(CheckMenuItem, bool)>,
) -> Result<Box<dyn IsMenuItem>, AppError> {
    Ok(match entry {
        Entry::Separator => Box::new(PredefinedMenuItem::separator()),
        Entry::Submenu(label, children) => {
            let menu = Submenu::new(label, !children.is_empty());
            for entry in children {
                menu.append(native_item(entry, commands, checks)?.as_ref())
                    .map_err(tray_error)?;
            }
            Box::new(menu)
        }
        Entry::Item {
            label,
            command,
            checked,
            enabled,
        } => {
            let accelerator = matches!(command, Some(TrayCommand::Quit))
                .then(|| Accelerator::new(Some(Modifiers::SUPER), Code::KeyQ));
            let item: Box<dyn IsMenuItem> = if let Some(checked) = checked {
                let item = CheckMenuItem::new(label, *enabled, *checked, accelerator);
                checks.push((item.clone(), *checked));
                Box::new(item)
            } else {
                Box::new(MenuItem::new(label, *enabled, accelerator))
            };
            if let Some(command) = command {
                commands.insert(item.id().as_ref().to_owned(), command.clone());
            }
            item
        }
    })
}

pub struct TrayService {
    tray: TrayIcon,
    commands: Arc<Mutex<HashMap<String, TrayCommand>>>,
    state: RefCell<Option<Arc<TrayMenuState>>>,
    checks: RefCell<Vec<(CheckMenuItem, bool)>>,
    clicked: Arc<AtomicBool>,
}

impl TrayService {
    pub fn new(on_command: impl Fn(TrayCommand) + Send + Sync + 'static) -> Result<Self, AppError> {
        let commands = Arc::new(Mutex::new(HashMap::<String, TrayCommand>::new()));
        let event_commands = commands.clone();
        let clicked = Arc::new(AtomicBool::new(false));
        let event_clicked = clicked.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let command = event_commands
                .lock()
                .expect("tray commands poisoned")
                .get(event.id().as_ref())
                .cloned();
            if let Some(command) = command {
                event_clicked.store(true, Ordering::Release);
                on_command(command);
            }
        }));
        let tray = TrayIconBuilder::new()
            .with_tooltip("Verge")
            .with_icon(load_icon()?)
            .with_icon_as_template(true)
            .build()
            .map_err(tray_error)?;
        let service = Self {
            tray,
            commands,
            state: RefCell::new(None),
            checks: RefCell::new(Vec::new()),
            clicked,
        };
        service.update(&TraySnapshot::default())?;
        Ok(service)
    }

    pub fn update(&self, snapshot: &TraySnapshot) -> Result<(), AppError> {
        // Traffic ticks never recreate the menu or its native objects.
        if self.state.borrow().as_ref() != Some(&snapshot.menu) {
            let menu = Menu::new();
            let mut commands = HashMap::new();
            let mut checks = Vec::new();
            for entry in menu_entries(&snapshot.menu) {
                menu.append(native_item(&entry, &mut commands, &mut checks)?.as_ref())
                    .map_err(tray_error)?;
            }
            *self.checks.borrow_mut() = checks;
            self.tray.set_menu(Some(Box::new(menu)));
            *self.commands.lock().expect("tray commands poisoned") = commands;
            *self.state.borrow_mut() = Some(snapshot.menu.clone());
        }
        // Native check items toggle before the backend responds; reassert confirmed state,
        // including failed writes whose snapshot remains unchanged.
        if self.clicked.swap(false, Ordering::AcqRel) {
            for (item, checked) in self.checks.borrow().iter() {
                item.set_checked(*checked);
            }
        }
        self.tray
            .set_tooltip(Some(format!(
                "Verge · ↑ {}/s   ↓ {}/s",
                format_bytes(snapshot.upload_bytes_per_second),
                format_bytes(snapshot.download_bytes_per_second)
            )))
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
    match bytes {
        1_048_576.. => format!("{:.1} MiB", bytes as f64 / 1_048_576.),
        1024.. => format!("{:.1} KiB", bytes as f64 / 1024.),
        _ => format!("{bytes} B"),
    }
}
fn tray_error(error: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::PlatformFailed, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn proxy_entries(state: &TrayMenuState) -> Vec<Entry> {
        let Entry::Submenu(_, entries) = menu_entries(state).remove(5) else {
            panic!("proxy submenu")
        };
        entries
    }
    #[test]
    fn modes_keep_group_membership_and_selection() {
        let mut state = TrayMenuState {
            mode: Some(RunMode::Rule),
            groups: vec![
                ProxyGroup {
                    name: "路线 / 亚洲".into(),
                    kind: "Selector".into(),
                    selected: Some("A:/🛰".into()),
                    members: vec!["A:/🛰".into(), "B".into()],
                },
                ProxyGroup {
                    name: "GLOBAL".into(),
                    kind: "Selector".into(),
                    selected: Some("B".into()),
                    members: vec!["B".into()],
                },
            ],
            ..Default::default()
        };
        let entries = proxy_entries(&state);
        let Entry::Submenu(_, members) = &entries[0] else {
            panic!("rule group")
        };
        assert_eq!(entries.len(), 1);
        assert!(
            matches!(&members[0], Entry::Item { checked: Some(true), command: Some(TrayCommand::SelectProxy { group, proxy }), .. } if group == "路线 / 亚洲" && proxy == "A:/🛰")
        );
        state.mode = Some(RunMode::Global);
        assert!(matches!(
            &proxy_entries(&state)[0],
            Entry::Item {
                checked: Some(true),
                ..
            }
        ));
        state.mode = Some(RunMode::Direct);
        assert!(proxy_entries(&state).is_empty());
    }
    #[test]
    fn offline_menu_disables_writes_and_preserves_recovery() {
        let mut state = TrayMenuState::default();
        assert!(matches!(
            &menu_entries(&state)[7],
            Entry::Item { enabled: false, .. }
        ));
        state.system_proxy_enabled = true;
        assert!(matches!(
            &menu_entries(&state)[7],
            Entry::Item {
                enabled: true,
                checked: Some(true),
                ..
            }
        ));
        assert_eq!(format_bytes(2048), "2.0 KiB");
    }
}
