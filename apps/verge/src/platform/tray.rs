use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::domain::{AppError, ErrorCode, ProfileId, ProxyGroup, RunMode};
use crate::i18n::{self, Lang, tr};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{
        CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
        accelerator::{Accelerator, Code, Modifiers},
    },
};

const TRAY_ICON: &[u8] = include_bytes!("../../../../assets/icons/tray-logo.png");
const SYSTEM_PROXY_ICON: &[u8] = include_bytes!("../../../../assets/icons/tray/system-proxy.png");
const TUN_ICON: &[u8] = include_bytes!("../../../../assets/icons/tray/tun.png");
const LEFT_EAR: &[u8] = include_bytes!("../../../../assets/icons/tray/left-ear.png");
const RIGHT_EAR: &[u8] = include_bytes!("../../../../assets/icons/tray/right-ear.png");
const SETTLE_EARS: &[u8] = include_bytes!("../../../../assets/icons/tray/settle.png");
const TRAY_ICON_SIZE: f64 = 18.0;
const EAR_HEIGHT: u32 = 8;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EarFrame {
    Rest,
    Left,
    Right,
    Settle,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum TrayIndicator {
    #[default]
    None,
    SystemProxy,
    Tun,
}

impl TrayIndicator {
    pub fn from_state(system_proxy: bool, tun: bool) -> Self {
        if tun {
            Self::Tun
        } else if system_proxy {
            Self::SystemProxy
        } else {
            Self::None
        }
    }
}

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
    pub indicator: TrayIndicator,
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
    let lang = Lang::from_code(&state.language);
    let mode_label = |mode| match mode {
        Some(RunMode::Rule) => tr(lang, "tray.mode.rule"),
        Some(RunMode::Global) => tr(lang, "tray.mode.global"),
        Some(RunMode::Direct) => tr(lang, "tray.mode.direct"),
        None => tr(lang, "tray.mode.offline"),
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
            if crate::identity::AppChannel::current().is_dev() {
                "Verge Dev"
            } else {
                tr(lang, "tray.show")
            },
            TrayCommand::ShowMainWindow,
            true,
        ),
        Entry::Separator,
        Entry::Submenu(
            i18n::fmt(
                lang,
                "tray.outbound_mode_title",
                &[
                    ("mode", tr(lang, "tray.outbound_mode")),
                    ("value", mode_label(state.mode)),
                ],
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
        Entry::Submenu(tr(lang, "tray.profiles").into(), profiles),
        Entry::Submenu(tr(lang, "tray.proxies").into(), proxies),
        Entry::Separator,
        Entry::check(
            tr(lang, "tray.system_proxy"),
            TrayCommand::ToggleSystemProxy,
            state.system_proxy_enabled,
            !crate::identity::AppChannel::current().is_dev()
                && (state.system_proxy_enabled || state.can_enable_system_proxy),
        ),
        Entry::Separator,
        Entry::Submenu(
            tr(lang, "tray.open_directory").into(),
            vec![
                Entry::action(
                    tr(lang, "tray.directory.data"),
                    TrayCommand::OpenDirectory(TrayDirectory::Data),
                    true,
                ),
                Entry::action(
                    tr(lang, "tray.directory.profiles"),
                    TrayCommand::OpenDirectory(TrayDirectory::Profiles),
                    true,
                ),
                Entry::action(
                    tr(lang, "tray.directory.logs"),
                    TrayCommand::OpenDirectory(TrayDirectory::Logs),
                    true,
                ),
            ],
        ),
        Entry::Submenu(
            tr(lang, "tray.more").into(),
            vec![
                Entry::action(
                    tr(lang, "tray.update_profile"),
                    TrayCommand::UpdateCurrentProfile,
                    state.can_update_profile,
                ),
                Entry::action(
                    tr(lang, "tray.restart_core"),
                    TrayCommand::RestartCore,
                    state.selected_profile.is_some(),
                ),
                Entry::action(
                    tr(lang, "tray.restart_app"),
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
        Entry::action(tr(lang, "tray.quit"), TrayCommand::Quit, true),
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
                .then(|| Accelerator::new(Modifiers::META, Code::KeyQ));
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
    icons: HashMap<(TrayIndicator, EarFrame), Icon>,
    indicator: Cell<TrayIndicator>,
    animation_generation: Cell<u64>,
    commands: Arc<Mutex<HashMap<String, TrayCommand>>>,
    state: RefCell<Option<Arc<TrayMenuState>>>,
    checks: RefCell<Vec<(CheckMenuItem, bool)>>,
    clicked: Arc<AtomicBool>,
}

impl TrayService {
    pub fn tray_id(&self) -> &tray_icon::TrayIconId {
        self.tray.id()
    }

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
            .with_tooltip(crate::identity::AppChannel::current().name())
            .with_icon_templated(load_icon()?)
            .build()
            .map_err(tray_error)?;
        let mut icons = HashMap::new();
        for indicator in [
            TrayIndicator::None,
            TrayIndicator::SystemProxy,
            TrayIndicator::Tun,
        ] {
            for frame in [
                EarFrame::Rest,
                EarFrame::Left,
                EarFrame::Right,
                EarFrame::Settle,
            ] {
                icons.insert((indicator, frame), render_icon(indicator, frame)?);
            }
        }
        let service = Self {
            tray,
            icons,
            indicator: Cell::new(TrayIndicator::None),
            animation_generation: Cell::new(0),
            commands,
            state: RefCell::new(None),
            checks: RefCell::new(Vec::new()),
            clicked,
        };
        service.set_icon(TrayIndicator::None, EarFrame::Rest)?;
        if crate::identity::AppChannel::current().is_dev() {
            service.tray.set_title(Some("Dev"));
        }
        service.update(&TraySnapshot::default())?;
        Ok(service)
    }

    pub fn update(&self, snapshot: &TraySnapshot) -> Result<(), AppError> {
        if self.indicator.get() != snapshot.indicator {
            self.animation_generation
                .set(self.animation_generation.get().wrapping_add(1));
            self.set_icon(snapshot.indicator, EarFrame::Rest)?;
            self.indicator.set(snapshot.indicator);
        }
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
                "{} · ↑ {}/s   ↓ {}/s",
                crate::identity::AppChannel::current().name(),
                format_bytes(snapshot.upload_bytes_per_second),
                format_bytes(snapshot.download_bytes_per_second)
            )))
            .map_err(tray_error)
    }

    fn set_icon(&self, indicator: TrayIndicator, frame: EarFrame) -> Result<(), AppError> {
        let icon = self.icons.get(&(indicator, frame)).ok_or_else(|| {
            AppError::new(ErrorCode::InvalidInput, "missing tray icon animation frame")
        })?;
        self.tray
            .set_icon_templated(Some(icon.clone()))
            .map_err(tray_error)?;
        // tray-icon 0.26 recreates NSImage at 22pt each time; retain all 32 source pixels.
        let marker = objc2::MainThreadMarker::new().expect("tray requires the main thread");
        let button = self
            .tray
            .ns_status_item()
            .and_then(|item| item.button(marker))
            .ok_or_else(|| tray_error("tray status button unavailable"))?;
        let image = button
            .image()
            .ok_or_else(|| tray_error("tray icon image unavailable"))?;
        image.setSize(objc2_foundation::NSSize::new(
            TRAY_ICON_SIZE,
            TRAY_ICON_SIZE,
        ));
        button.setImage(Some(&image));
        Ok(())
    }

    pub fn start_click_animation(&self) -> Result<u64, AppError> {
        let generation = self.animation_generation.get().wrapping_add(1);
        self.animation_generation.set(generation);
        self.set_icon(self.indicator.get(), EarFrame::Left)?;
        Ok(generation)
    }

    pub fn advance_click_animation(&self, generation: u64, frame: EarFrame) {
        if self.animation_generation.get() == generation
            && let Err(error) = self.set_icon(self.indicator.get(), frame)
        {
            eprintln!("[verge] tray animation: {}", error.message);
        }
    }
}

fn icon_image(indicator: TrayIndicator, frame: EarFrame) -> Result<image::RgbaImage, AppError> {
    let base = match indicator {
        TrayIndicator::None => TRAY_ICON,
        TrayIndicator::SystemProxy => SYSTEM_PROXY_ICON,
        TrayIndicator::Tun => TUN_ICON,
    };
    let mut image = image::load_from_memory(base)
        .map_err(tray_error)?
        .into_rgba8();
    if frame != EarFrame::Rest {
        let ears = match frame {
            EarFrame::Left => LEFT_EAR,
            EarFrame::Right => RIGHT_EAR,
            EarFrame::Settle => SETTLE_EARS,
            EarFrame::Rest => unreachable!(),
        };
        let ears = image::load_from_memory(ears)
            .map_err(tray_error)?
            .into_rgba8();
        if image.dimensions() != ears.dimensions() {
            return Err(tray_error("tray icon frames have different dimensions"));
        }
        // Only the ears differ in the preview. Keep the status cutout and head untouched.
        for y in 0..EAR_HEIGHT {
            for x in 0..image.width() {
                image.put_pixel(x, y, *ears.get_pixel(x, y));
            }
        }
    }
    Ok(image)
}

fn render_icon(indicator: TrayIndicator, frame: EarFrame) -> Result<Icon, AppError> {
    let image = icon_image(indicator, frame)?;
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height).map_err(tray_error)
}

fn load_icon() -> Result<Icon, AppError> {
    render_icon(TrayIndicator::None, EarFrame::Rest)
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

    #[test]
    fn indicator_prefers_active_tun_and_keeps_original_bitmap_resolution() {
        assert_eq!(TrayIndicator::from_state(false, false), TrayIndicator::None);
        assert_eq!(
            TrayIndicator::from_state(true, false),
            TrayIndicator::SystemProxy
        );
        assert_eq!(TrayIndicator::from_state(false, true), TrayIndicator::Tun);
        assert_eq!(TrayIndicator::from_state(true, true), TrayIndicator::Tun);

        let original = image::load_from_memory(TRAY_ICON).unwrap().into_rgba8();
        assert_eq!(
            icon_image(TrayIndicator::None, EarFrame::Rest).unwrap(),
            original
        );
        assert_ne!(
            icon_image(TrayIndicator::SystemProxy, EarFrame::Rest).unwrap(),
            icon_image(TrayIndicator::Tun, EarFrame::Rest).unwrap()
        );
        for indicator in [
            TrayIndicator::None,
            TrayIndicator::SystemProxy,
            TrayIndicator::Tun,
        ] {
            let rest = icon_image(indicator, EarFrame::Rest).unwrap();
            for frame in [EarFrame::Left, EarFrame::Right, EarFrame::Settle] {
                let image = icon_image(indicator, frame).unwrap();
                assert_eq!(image.dimensions(), original.dimensions());
                assert_ne!(image, rest);
                for y in EAR_HEIGHT..image.height() {
                    for x in 0..image.width() {
                        assert_eq!(image.get_pixel(x, y), rest.get_pixel(x, y));
                    }
                }
            }
        }
        for indicator in [TrayIndicator::SystemProxy, TrayIndicator::Tun] {
            let image = icon_image(indicator, EarFrame::Rest).unwrap();
            assert_eq!(image.dimensions(), original.dimensions());
            let mut cutout_pixels = 0;
            // Only the lower central glyph is cut out; the outline and ears stay intact.
            for (x, y, source) in original.enumerate_pixels() {
                let marked = image.get_pixel(x, y);
                assert!(marked.0[3] <= source.0[3]);
                if marked.0[3] != source.0[3] {
                    assert!((11..=21).contains(&x) && (16..=27).contains(&y));
                    cutout_pixels += 1;
                }
            }
            assert!(cutout_pixels > 0);
            for y in 0..EAR_HEIGHT {
                for x in 0..image.width() {
                    assert_eq!(image.get_pixel(x, y), original.get_pixel(x, y));
                }
            }
        }
    }

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
                enabled,
                checked: Some(true),
                ..
            } if *enabled == !crate::identity::AppChannel::current().is_dev()
        ));
        assert_eq!(format_bytes(2048), "2.0 KiB");
    }
    #[test]
    fn tray_uses_embedded_locale() {
        let state = TrayMenuState {
            language: "zh-CN".into(),
            ..Default::default()
        };
        assert!(
            matches!(&menu_entries(&state)[2], Entry::Submenu(title, _) if title == "出站模式（内核未运行）")
        );
        assert!(matches!(&menu_entries(&state)[5], Entry::Submenu(title, _) if title == "代理"));
    }
}
