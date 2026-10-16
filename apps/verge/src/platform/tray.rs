use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::domain::{AppError, ErrorCode, ProfileId, ProxyGroup, RunMode};
use crate::i18n::{self, Lang, tr};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{
        CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, MenuItemKind, PredefinedMenuItem,
        Submenu,
        accelerator::{Accelerator, Code, Modifiers},
    },
};

#[path = "tray_animation.rs"]
mod animation;
use animation::Frame;

const TRAY_ICON: &[u8] = include_bytes!("../../../../assets/icons/tray-logo.png");
const SYSTEM_PROXY_ICON: &[u8] = include_bytes!("../../../../assets/icons/tray/system-proxy.png");
const TUN_ICON: &[u8] = include_bytes!("../../../../assets/icons/tray/tun.png");
const TRAY_ICON_SIZE: f64 = 18.0;

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
    SetSystemProxy { enabled: bool, request_id: u64 },
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
    pub completed_system_proxy_request: u64,
}

impl TrayMenuState {
    fn same_structure(&self, other: &Self) -> bool {
        self.language == other.language
            && self.mode == other.mode
            && self.profiles == other.profiles
            && self.selected_profile == other.selected_profile
            && self.can_update_profile == other.can_update_profile
            && self.can_restart_application == other.can_restart_application
            && self.groups == other.groups
    }
}

#[derive(Default)]
struct ProxyInteraction {
    issued: u64,
    pending: Option<u64>,
}

impl ProxyInteraction {
    fn begin(&mut self) -> Option<u64> {
        if self.pending.is_some() {
            return None;
        }
        self.issued += 1;
        self.pending = Some(self.issued);
        self.pending
    }

    fn complete(&mut self, completed: u64) {
        if self.pending.is_some_and(|id| completed >= id) {
            self.pending = None;
        }
    }
}

#[derive(Default)]
struct PendingTrayUpdate(Mutex<Option<TraySnapshot>>);

impl PendingTrayUpdate {
    fn replace(&self, snapshot: TraySnapshot) -> bool {
        self.0
            .lock()
            .expect("tray update poisoned")
            .replace(snapshot)
            .is_none()
    }

    fn take(&self) -> Option<TraySnapshot> {
        self.0.lock().expect("tray update poisoned").take()
    }
}

#[derive(Clone)]
pub struct TrayUpdateSender {
    tray: Arc<dispatch2::MainThreadBound<TrayService>>,
    pending: Arc<PendingTrayUpdate>,
}

impl TrayUpdateSender {
    pub fn new(tray: Arc<dispatch2::MainThreadBound<TrayService>>) -> Self {
        Self {
            tray,
            pending: Arc::default(),
        }
    }

    pub fn send(&self, snapshot: TraySnapshot) {
        // Keep only the newest projection, with at most one queued run-loop callback.
        if !self.pending.replace(snapshot) {
            return;
        }
        let tray = self.tray.clone();
        let pending = self.pending.clone();
        on_tray_run_loop(move || {
            let marker = objc2::MainThreadMarker::new().expect("tray update requires main thread");
            if let Some(snapshot) = pending.take()
                && let Err(error) = tray.get(marker).update(&snapshot)
            {
                eprintln!("[verge] tray update: {}", error.message);
            }
        });
    }

    pub fn begin_shutdown(&self) {
        let tray = self.tray.clone();
        on_tray_run_loop(move || {
            let marker =
                objc2::MainThreadMarker::new().expect("tray shutdown requires main thread");
            tray.get(marker).begin_shutdown();
        });
    }
}

fn on_tray_run_loop(callback: impl Fn() + Send + 'static) {
    on_main_run_loop(callback, true);
}

fn on_main_run_loop(callback: impl Fn() + Send + 'static, during_tracking: bool) {
    use objc2_app_kit::NSEventTrackingRunLoopMode;
    use objc2_foundation::{NSArray, NSDefaultRunLoopMode, NSRunLoop};

    let block = block2::RcBlock::new(callback);
    let run_loop = NSRunLoop::mainRunLoop();
    // The callback is Send. Status updates run during tracking; structural work
    // waits for the normal loop so the selected item's action can finish first.
    unsafe {
        let modes = if during_tracking {
            NSArray::from_slice(&[NSDefaultRunLoopMode, NSEventTrackingRunLoopMode])
        } else {
            NSArray::from_slice(&[NSDefaultRunLoopMode])
        };
        run_loop.performInModes_block(&modes, &block);
    }
    run_loop.getCFRunLoop().wake_up();
}

#[derive(Default)]
struct MenuTracking {
    active: bool,
    generation: u64,
}

impl MenuTracking {
    fn begin(&mut self) {
        self.active = true;
        self.generation = self.generation.wrapping_add(1);
    }

    fn finish(&mut self, generation: u64) -> bool {
        if self.generation != generation {
            return false;
        }
        self.active = false;
        true
    }
}

struct TrackingObserver(
    objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_foundation::NSObjectProtocol>>,
);

impl Drop for TrackingObserver {
    fn drop(&mut self) {
        unsafe {
            objc2_foundation::NSNotificationCenter::defaultCenter()
                .removeObserver((*self.0).as_ref())
        };
    }
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
            TrayCommand::SetSystemProxy {
                enabled: !state.system_proxy_enabled,
                request_id: 0,
            },
            state.system_proxy_enabled,
            can_change_proxy(state),
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

fn can_change_proxy(state: &TrayMenuState) -> bool {
    !crate::identity::AppChannel::current().is_dev()
        && (state.system_proxy_enabled || state.can_enable_system_proxy)
}

fn native_item(
    entry: &Entry,
    commands: &mut HashMap<String, TrayCommand>,
    checks: &mut Vec<(CheckMenuItem, bool)>,
    proxy_item: &mut Option<CheckMenuItem>,
) -> Result<Box<dyn IsMenuItem>, AppError> {
    Ok(match entry {
        Entry::Separator => Box::new(PredefinedMenuItem::separator()),
        Entry::Submenu(label, children) => {
            let menu = Submenu::new(label, !children.is_empty());
            for entry in children {
                menu.append(native_item(entry, commands, checks, proxy_item)?.as_ref())
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
                if matches!(command, Some(TrayCommand::SetSystemProxy { .. })) {
                    *proxy_item = Some(item.clone());
                } else {
                    checks.push((item.clone(), *checked));
                }
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
    menu: RefCell<Menu>,
    icons: HashMap<(TrayIndicator, Frame), Icon>,
    indicator: Cell<TrayIndicator>,
    animation_generation: Cell<u64>,
    commands: RefCell<HashMap<String, TrayCommand>>,
    state: RefCell<Option<Arc<TrayMenuState>>>,
    checks: RefCell<Vec<(CheckMenuItem, bool)>>,
    clicked: Cell<bool>,
    shutting_down: Cell<bool>,
    proxy_item: RefCell<Option<CheckMenuItem>>,
    proxy_interaction: RefCell<ProxyInteraction>,
    tracking: RefCell<MenuTracking>,
    deferred_menu: RefCell<Option<Arc<TrayMenuState>>>,
    tracking_observers: RefCell<Vec<TrackingObserver>>,
}

impl TrayService {
    pub fn tray_id(&self) -> &tray_icon::TrayIconId {
        self.tray.id()
    }

    pub fn bind(
        self,
        on_command: impl Fn(TrayCommand) + Send + Sync + 'static,
    ) -> Arc<dispatch2::MainThreadBound<Self>> {
        let marker = objc2::MainThreadMarker::new().expect("tray requires main thread");
        let bound = Arc::new(dispatch2::MainThreadBound::new(self, marker));
        let weak = Arc::downgrade(&bound);
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(bound) = weak.upgrade()
                && let Some(command) =
                    bound.get_on_main(|tray| tray.menu_command(event.id().as_ref()))
            {
                on_command(command);
            }
        }));
        Self::observe_tracking(&bound, marker);
        bound
    }

    fn observe_tracking(
        bound: &Arc<dispatch2::MainThreadBound<Self>>,
        marker: objc2::MainThreadMarker,
    ) {
        use objc2_app_kit::{NSMenuDidBeginTrackingNotification, NSMenuDidEndTrackingNotification};
        use objc2_foundation::{NSNotification, NSNotificationCenter};
        use tray_icon::menu::ContextMenu;

        for (name, begin) in unsafe {
            [
                (NSMenuDidBeginTrackingNotification, true),
                (NSMenuDidEndTrackingNotification, false),
            ]
        } {
            let weak = Arc::downgrade(bound);
            let block =
                block2::RcBlock::new(move |notification: std::ptr::NonNull<NSNotification>| {
                    let Some(marker) = objc2::MainThreadMarker::new() else {
                        return;
                    };
                    let Some(bound) = weak.upgrade() else {
                        return;
                    };
                    let tray = bound.get(marker);
                    let Some(object) = (unsafe { notification.as_ref() }).object() else {
                        return;
                    };
                    if !std::ptr::eq(
                        (&*object as *const objc2::runtime::AnyObject).cast(),
                        tray.menu.borrow().ns_menu(),
                    ) {
                        return;
                    }
                    if begin {
                        tray.tracking.borrow_mut().begin();
                    } else {
                        let generation = tray.tracking.borrow().generation;
                        let weak = weak.clone();
                        // didEndTracking can precede the selected item's action. Keep the old
                        // objects and command IDs until Cocoa unwinds to the normal run loop.
                        on_main_run_loop(
                            move || {
                                if let Some(bound) = weak.upgrade() {
                                    let marker = objc2::MainThreadMarker::new()
                                        .expect("menu close requires main thread");
                                    let tray = bound.get(marker);
                                    let finished = tray.tracking.borrow_mut().finish(generation);
                                    if finished {
                                        let deferred = tray.deferred_menu.borrow_mut().take();
                                        if let Some(menu) = deferred
                                            && let Err(error) = tray.update_menu(&menu)
                                        {
                                            eprintln!(
                                                "[verge] deferred tray menu: {}",
                                                error.message
                                            );
                                        }
                                    }
                                }
                            },
                            false,
                        );
                    }
                });
            let observer =
                unsafe {
                    NSNotificationCenter::defaultCenter()
                        .addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
                };
            bound
                .get(marker)
                .tracking_observers
                .borrow_mut()
                .push(TrackingObserver(observer));
        }
    }

    fn menu_command(&self, id: &str) -> Option<TrayCommand> {
        if self.shutting_down.get() {
            return None;
        }
        let mut command = self.commands.borrow().get(id)?.clone();
        if matches!(command, TrayCommand::Quit) {
            self.begin_shutdown();
        } else if matches!(command, TrayCommand::SetSystemProxy { .. }) {
            let state = self.state.borrow();
            let state = state.as_ref()?;
            if !can_change_proxy(state) {
                return None;
            }
            let request_id = self.proxy_interaction.borrow_mut().begin()?;
            command = TrayCommand::SetSystemProxy {
                enabled: !state.system_proxy_enabled,
                request_id,
            };
            self.update_proxy_item(state);
        } else {
            self.clicked.set(true);
        }
        Some(command)
    }

    pub fn new() -> Result<Self, AppError> {
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
            for frame in Frame::all() {
                icons.insert((indicator, frame), render_icon(indicator, frame)?);
            }
        }
        let service = Self {
            tray,
            menu: RefCell::new(Menu::new()),
            icons,
            indicator: Cell::new(TrayIndicator::None),
            animation_generation: Cell::new(0),
            commands: RefCell::default(),
            state: RefCell::new(None),
            checks: RefCell::new(Vec::new()),
            clicked: Cell::new(false),
            shutting_down: Cell::new(false),
            proxy_item: RefCell::new(None),
            proxy_interaction: RefCell::new(ProxyInteraction::default()),
            tracking: RefCell::new(MenuTracking::default()),
            deferred_menu: RefCell::new(None),
            tracking_observers: RefCell::new(Vec::new()),
        };
        service.set_icon(TrayIndicator::None, Frame::REST)?;
        if crate::identity::AppChannel::current().is_dev() {
            service.tray.set_title(Some("Dev"));
        }
        service.update(&TraySnapshot::default())?;
        Ok(service)
    }

    pub fn update(&self, snapshot: &TraySnapshot) -> Result<(), AppError> {
        if self.shutting_down.get() {
            return Ok(());
        }
        let was_pending = self.proxy_interaction.borrow().pending.is_some();
        self.proxy_interaction
            .borrow_mut()
            .complete(snapshot.completed_system_proxy_request);
        if self.indicator.get() != snapshot.indicator {
            self.animation_generation
                .set(self.animation_generation.get().wrapping_add(1));
            self.set_icon(snapshot.indicator, Frame::REST)?;
            self.indicator.set(snapshot.indicator);
        }
        self.update_menu(&snapshot.menu)?;
        if was_pending {
            self.update_proxy_item(&snapshot.menu);
        }
        // Native check items toggle before the backend responds; reassert confirmed state,
        // including failed writes whose snapshot remains unchanged.
        if self.clicked.replace(false) {
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

    fn update_menu(&self, desired: &Arc<TrayMenuState>) -> Result<(), AppError> {
        if self.shutting_down.get() {
            return Ok(());
        }
        let changed = self.state.borrow().as_ref() != Some(desired);
        let rebuild = changed
            && self
                .state
                .borrow()
                .as_ref()
                .is_none_or(|state| !state.same_structure(desired));
        if rebuild && self.tracking.borrow().active {
            *self.deferred_menu.borrow_mut() = Some(desired.clone());
            self.update_proxy_item(desired);
            if let Some(state) = self.state.borrow_mut().as_mut() {
                let state = Arc::make_mut(state);
                state.system_proxy_enabled = desired.system_proxy_enabled;
                state.can_enable_system_proxy = desired.can_enable_system_proxy;
            }
            return Ok(());
        }
        self.deferred_menu.borrow_mut().take();
        if rebuild {
            let menu = Menu::new();
            let mut commands = HashMap::new();
            let mut checks = Vec::new();
            let mut proxy_item = None;
            for entry in menu_entries(desired) {
                menu.append(
                    native_item(&entry, &mut commands, &mut checks, &mut proxy_item)?.as_ref(),
                )
                .map_err(tray_error)?;
            }
            *self.checks.borrow_mut() = checks;
            *self.proxy_item.borrow_mut() = proxy_item;
            self.tray.set_menu(Some(Box::new(menu.clone())));
            *self.menu.borrow_mut() = menu;
            *self.commands.borrow_mut() = commands;
        }
        if changed || rebuild {
            self.update_proxy_item(desired);
        }
        *self.state.borrow_mut() = Some(desired.clone());
        Ok(())
    }

    fn begin_shutdown(&self) {
        if self.shutting_down.replace(true) {
            return;
        }
        self.deferred_menu.borrow_mut().take();
        self.animation_generation
            .set(self.animation_generation.get().wrapping_add(1));
        let lang = self
            .state
            .borrow()
            .as_ref()
            .map_or(Lang::En, |state| Lang::from_code(&state.language));
        let label = tr(lang, "tray.quitting");
        // Retain the visible menu and command IDs until Cocoa finishes dispatching this click.
        for item in self.menu.borrow().items() {
            match item {
                MenuItemKind::MenuItem(item) => {
                    if matches!(
                        self.commands.borrow().get(item.id().as_ref()),
                        Some(TrayCommand::Quit)
                    ) {
                        item.set_text(label);
                    }
                    item.set_enabled(false);
                }
                MenuItemKind::Check(item) => item.set_enabled(false),
                MenuItemKind::Submenu(item) => item.set_enabled(false),
                MenuItemKind::Icon(item) => item.set_enabled(false),
                MenuItemKind::Predefined(_) => {}
            }
        }
        self.tray.set_title(Some(label));
        if let Err(error) = self.tray.set_tooltip(Some(label)) {
            eprintln!("[verge] tray shutdown feedback: {error}");
        }
    }

    fn update_proxy_item(&self, state: &TrayMenuState) {
        if let Some(item) = self.proxy_item.borrow().as_ref() {
            let pending = self.proxy_interaction.borrow().pending.is_some();
            let lang = Lang::from_code(&state.language);
            let label = tr(lang, "tray.system_proxy");
            item.set_text(if pending {
                format!("{label} · {}", tr(lang, "proxy.switching"))
            } else {
                label.into()
            });
            item.set_enabled(!pending && can_change_proxy(state));
            // A pending action is not a successful OS write. Undo muda's automatic toggle.
            item.set_checked(state.system_proxy_enabled);
        }
    }

    fn set_icon(&self, indicator: TrayIndicator, frame: Frame) -> Result<(), AppError> {
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

    fn start_click_animation(&self) -> Result<u64, AppError> {
        let generation = self.animation_generation.get().wrapping_add(1);
        self.animation_generation.set(generation);
        self.set_icon(self.indicator.get(), Frame::REST)?;
        Ok(generation)
    }

    fn advance_click_animation(&self, generation: u64, frame: Frame) {
        if self.animation_generation.get() == generation
            && let Err(error) = self.set_icon(self.indicator.get(), frame)
        {
            eprintln!("[verge] tray animation: {}", error.message);
        }
    }

    pub fn animate_click(bound: &Arc<dispatch2::MainThreadBound<Self>>) {
        use objc2_app_kit::{NSEventTrackingRunLoopMode, NSWorkspace};
        use objc2_foundation::{NSDefaultRunLoopMode, NSRunLoop, NSTimer};

        let marker = objc2::MainThreadMarker::new().expect("tray requires the main thread");
        let tray = bound.get(marker);
        if tray.shutting_down.get() {
            return;
        }
        let generation = match tray.start_click_animation() {
            Ok(generation) => generation,
            Err(error) => {
                eprintln!("[verge] tray animation: {}", error.message);
                return;
            }
        };
        if NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion() {
            return;
        }
        let weak = Arc::downgrade(bound);
        let started = std::time::Instant::now();
        let block = block2::RcBlock::new(move |timer: std::ptr::NonNull<NSTimer>| {
            let timer = unsafe { timer.as_ref() };
            let Some(bound) = weak.upgrade() else {
                timer.invalidate();
                return;
            };
            let marker = objc2::MainThreadMarker::new().expect("tray timer requires main thread");
            let tray = bound.get(marker);
            if tray.animation_generation.get() != generation {
                timer.invalidate();
                return;
            }
            let elapsed = started
                .elapsed()
                .as_millis()
                .min(u128::from(animation::DURATION_MS)) as u64;
            tray.advance_click_animation(generation, Frame::at_elapsed(elapsed));
            if elapsed >= animation::DURATION_MS {
                timer.invalidate();
            }
        });
        let timer = unsafe {
            NSTimer::timerWithTimeInterval_repeats_block(
                animation::FRAME_INTERVAL_MS as f64 / 1000.0,
                true,
                &block,
            )
        };
        // The native menu uses a nested run loop; animation must run in both modes.
        unsafe {
            let run_loop = NSRunLoop::mainRunLoop();
            run_loop.addTimer_forMode(&timer, NSDefaultRunLoopMode);
            run_loop.addTimer_forMode(&timer, NSEventTrackingRunLoopMode);
        }
    }
}

fn icon_image(indicator: TrayIndicator, frame: Frame) -> Result<image::RgbaImage, AppError> {
    let base = match indicator {
        TrayIndicator::None => TRAY_ICON,
        TrayIndicator::SystemProxy => SYSTEM_PROXY_ICON,
        TrayIndicator::Tun => TUN_ICON,
    };
    let image = image::load_from_memory(base)
        .map_err(tray_error)?
        .into_rgba8();
    if image.dimensions() != (32, 32) {
        return Err(tray_error(
            "tray animation requires the original 32px artwork",
        ));
    }
    Ok(animation::render(&image, frame))
}

fn render_icon(indicator: TrayIndicator, frame: Frame) -> Result<Icon, AppError> {
    let image = icon_image(indicator, frame)?;
    let (width, height) = image.dimensions();
    Icon::from_rgba(image.into_raw(), width, height).map_err(tray_error)
}

fn load_icon() -> Result<Icon, AppError> {
    render_icon(TrayIndicator::None, Frame::REST)
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
    fn stale_menu_close_cannot_release_a_reopened_menu() {
        let mut tracking = MenuTracking::default();
        tracking.begin();
        let closing = tracking.generation;
        assert!(tracking.active);
        tracking.begin();
        assert!(!tracking.finish(closing));
        assert!(tracking.active);
        assert!(tracking.finish(tracking.generation));
        assert!(!tracking.active);
    }

    #[test]
    fn proxy_pending_rejects_duplicate_clicks_and_stale_completions() {
        let mut interaction = ProxyInteraction::default();
        assert_eq!(interaction.begin(), Some(1));
        assert_eq!(interaction.begin(), None);
        interaction.complete(0);
        assert_eq!(interaction.pending, Some(1));
        interaction.complete(1);
        assert_eq!(interaction.pending, None);
        assert_eq!(interaction.begin(), Some(2));
        interaction.complete(1);
        assert_eq!(interaction.pending, Some(2));
        interaction.complete(2);
        assert_eq!(interaction.pending, None);
    }

    #[test]
    fn tray_delivery_coalesces_to_latest_state_without_losing_completion() {
        let pending = PendingTrayUpdate::default();
        assert!(pending.replace(TraySnapshot::default()));
        assert!(!pending.replace(TraySnapshot {
            indicator: TrayIndicator::SystemProxy,
            completed_system_proxy_request: 1,
            ..Default::default()
        }));
        let newest = pending.take().unwrap();
        assert_eq!(newest.indicator, TrayIndicator::SystemProxy);
        assert_eq!(newest.completed_system_proxy_request, 1);
        assert!(pending.take().is_none());
        assert!(pending.replace(TraySnapshot::default()));
    }

    #[test]
    fn system_proxy_updates_keep_menu_structure_and_use_explicit_intent() {
        let mut state = TrayMenuState {
            can_enable_system_proxy: true,
            ..Default::default()
        };
        let initial = state.clone();
        assert!(matches!(
            &menu_entries(&state)[7],
            Entry::Item {
                command: Some(TrayCommand::SetSystemProxy { enabled: true, .. }),
                ..
            }
        ));
        state.system_proxy_enabled = true;
        state.can_enable_system_proxy = false;
        assert!(state.same_structure(&initial));
        assert!(matches!(
            &menu_entries(&state)[7],
            Entry::Item {
                command: Some(TrayCommand::SetSystemProxy { enabled: false, .. }),
                ..
            }
        ));
        state.language = "zh-CN".into();
        assert!(!state.same_structure(&initial));
    }

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
            icon_image(TrayIndicator::None, Frame::REST).unwrap(),
            original
        );
        assert_ne!(
            icon_image(TrayIndicator::SystemProxy, Frame::REST).unwrap(),
            icon_image(TrayIndicator::Tun, Frame::REST).unwrap()
        );
        for indicator in [
            TrayIndicator::None,
            TrayIndicator::SystemProxy,
            TrayIndicator::Tun,
        ] {
            let rest = icon_image(indicator, Frame::REST).unwrap();
            for frame in Frame::all().skip(1) {
                let image = icon_image(indicator, frame).unwrap();
                assert_eq!(image.dimensions(), original.dimensions());
                assert_ne!(image, rest);
                // The complete status region stays pixel-identical, including antialiasing.
                for y in 16..=27 {
                    for x in 11..=21 {
                        assert_eq!(image.get_pixel(x, y), rest.get_pixel(x, y));
                    }
                }
            }
        }
        for indicator in [TrayIndicator::SystemProxy, TrayIndicator::Tun] {
            let image = icon_image(indicator, Frame::REST).unwrap();
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
            for y in 0..8 {
                for x in 0..image.width() {
                    assert_eq!(image.get_pixel(x, y), original.get_pixel(x, y));
                }
            }
        }
    }

    #[test]
    fn animation_preserves_static_artwork_and_status_in_every_frame() {
        for (indicator, bytes) in [
            (TrayIndicator::None, TRAY_ICON),
            (TrayIndicator::SystemProxy, SYSTEM_PROXY_ICON),
            (TrayIndicator::Tun, TUN_ICON),
        ] {
            let original = image::load_from_memory(bytes).unwrap().into_rgba8();
            assert_eq!(icon_image(indicator, Frame::REST).unwrap(), original);
            for frame in Frame::all() {
                let plain = icon_image(TrayIndicator::None, frame).unwrap();
                let marked = icon_image(indicator, frame).unwrap();
                for (x, y, pixel) in marked.enumerate_pixels() {
                    assert!(pixel[3] <= plain.get_pixel(x, y)[3]);
                    if !(11..=21).contains(&x) || !(16..=27).contains(&y) {
                        assert_eq!(pixel[3], plain.get_pixel(x, y)[3]);
                    }
                }
            }
        }
    }

    #[test]
    fn animation_moves_ears_head_jaw_and_neck_without_shrinking_the_icon() {
        let original = icon_image(TrayIndicator::None, Frame::REST).unwrap();
        let alpha_mass = |image: &image::RgbaImage| -> i64 {
            image.pixels().map(|pixel| i64::from(pixel[3])).sum()
        };
        let bounds = |image: &image::RgbaImage| {
            image
                .enumerate_pixels()
                .filter(|(_, _, p)| p[3] >= 128)
                .fold((32u32, 32u32, 0u32, 0u32), |(x0, y0, x1, y1), (x, y, _)| {
                    (x0.min(x), y0.min(y), x1.max(x), y1.max(y))
                })
        };
        let original_bounds = bounds(&original);
        let original_mass = alpha_mass(&original);
        let mut changed_regions = [false; 4];
        for frame in Frame::all().skip(1) {
            let image = icon_image(TrayIndicator::None, frame).unwrap();
            let actual = bounds(&image);
            for (a, b) in [
                (actual.0, original_bounds.0),
                (actual.1, original_bounds.1),
                (actual.2, original_bounds.2),
                (actual.3, original_bounds.3),
            ] {
                assert!(a.abs_diff(b) <= 1, "outline changed size: {frame:?}");
            }
            assert!((alpha_mass(&image) - original_mass).abs() * 100 < original_mass * 5);
            for (i, (xs, ys)) in [
                (10..23, 0..8),
                (22..32, 9..18),
                (24..32, 18..22),
                (1..11, 18..29),
            ]
            .into_iter()
            .enumerate()
            {
                changed_regions[i] |= ys.into_iter().any(|y| {
                    xs.clone()
                        .any(|x| image.get_pixel(x, y)[3] != original.get_pixel(x, y)[3])
                });
            }
        }
        assert_eq!(changed_regions, [true; 4]);
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
