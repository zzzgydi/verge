//! Daemon-side tray projection. Slow OS/controller reads never run on traffic ticks.
use super::*;
use crate::domain::{ProfileSource, ProxySnapshot, RunMode, SystemProxyState};
use crate::platform::{TrayDirectory, TrayMenuState};

pub(super) struct RefreshResult {
    generation: u64,
    mode: Option<RunMode>,
    proxies: ProxySnapshot,
    system_proxy: Option<SystemProxyState>,
}

pub(super) struct TrayState {
    generation: u64,
    in_flight: bool,
    next_refresh: Instant,
    next_system_refresh: Instant,
    mode: Option<RunMode>,
    proxies: ProxySnapshot,
    system_proxy: Option<SystemProxyState>,
    menu: Arc<TrayMenuState>,
    menu_dirty: bool,
    last_sent: Option<TraySnapshot>,
}

impl TrayState {
    pub(super) fn new() -> Self {
        Self {
            generation: 0,
            in_flight: false,
            next_refresh: Instant::now(),
            next_system_refresh: Instant::now(),
            mode: None,
            proxies: ProxySnapshot::default(),
            system_proxy: None,
            menu: Arc::default(),
            menu_dirty: true,
            last_sent: None,
        }
    }

    pub(super) fn invalidate(&mut self) {
        self.menu_dirty = true;
        self.last_sent = None;
        self.generation = self.generation.wrapping_add(1);
        self.next_refresh = Instant::now();
        self.next_system_refresh = Instant::now();
    }

    pub(super) fn refresh(&mut self, backend: &Backend, events: &Sender<DaemonEvent>) {
        if self.in_flight || Instant::now() < self.next_refresh {
            return;
        }
        let runtime = backend.query_runtime().ok();
        let read_system = Instant::now() >= self.next_system_refresh;
        let services = backend.system_proxy.managed_services().to_vec();
        let recovery = backend.config.recovery_path.clone();
        let order = backend.profiles.proxy_group_order().unwrap_or_default();
        let generation = self.generation;
        let events = events.clone();
        self.in_flight = true;
        self.next_refresh = Instant::now() + Duration::from_secs(5);
        if read_system {
            self.next_system_refresh = Instant::now() + Duration::from_secs(30);
        }
        std::thread::spawn(move || {
            let mut mode = None;
            let mut proxies = ProxySnapshot::default();
            if let Some(mut runtime) = runtime {
                let mut handler = RuntimeCommandHandler::new(&mut runtime);
                if let Ok(result) = handler.execute(RuntimeCommand::GetMode)
                    && let RuntimeCommandOutput::Mode(value) = result.output
                {
                    mode = Some(value);
                }
                if mode.is_some()
                    && let Ok(result) = handler.execute(RuntimeCommand::ListProxyGroups)
                    && let RuntimeCommandOutput::ProxyGroups(value) = result.output
                {
                    proxies = value;
                }
            }
            let indices: std::collections::HashMap<_, _> = order
                .iter()
                .enumerate()
                .map(|(i, name)| (name.as_str(), i))
                .collect();
            proxies
                .groups
                .sort_by_key(|g| indices.get(g.name.as_str()).copied().unwrap_or(usize::MAX));
            let system_proxy = read_system
                .then(|| {
                    MacSystemProxy::new(ProcessRunner, recovery)
                        .state(&services)
                        .ok()
                })
                .flatten();
            let _ = events.send(DaemonEvent::TrayRefreshed(Box::new(RefreshResult {
                generation,
                mode,
                proxies,
                system_proxy,
            })));
        });
    }

    pub(super) fn accept(&mut self, result: RefreshResult, server: &IpcServer) {
        self.in_flight = false;
        // An OS read started before a write must never overwrite its newer state.
        if result.generation != self.generation {
            return;
        }
        if let Some(state) = result.system_proxy
            && self.system_proxy.as_ref() != Some(&state)
        {
            send_response(
                server,
                UiResponse::SystemProxy {
                    request: SystemProxyCommand::GetState,
                    result: Ok(SystemProxyCommandResult {
                        state: state.clone(),
                        summary: "System proxy state refreshed".into(),
                    }),
                },
            );
            self.system_proxy = Some(state);
        }
        if self.mode != result.mode
            && let Some(mode) = result.mode
        {
            send_response(
                server,
                UiResponse::Runtime {
                    request: RuntimeCommand::GetMode,
                    result: Ok(crate::domain::RuntimeCommandResult {
                        output: RuntimeCommandOutput::Mode(mode),
                        summary: "Mode refreshed".into(),
                    }),
                },
            );
        }
        self.menu_dirty = true;
        self.mode = result.mode;
        self.proxies = result.proxies;
    }

    pub(super) fn publish(&mut self, backend: &Backend, updates: &mpsc::SyncSender<TraySnapshot>) {
        if self.menu_dirty {
            let selected_profile = backend.profiles.selected().cloned();
            let menu = TrayMenuState {
                language: backend.settings.get().language.clone(),
                system_proxy_enabled: self
                    .system_proxy
                    .as_ref()
                    .is_some_and(SystemProxyState::unified_enabled),
                can_enable_system_proxy: backend.engine.is_some()
                    && backend.runtime_settings_snapshot().is_some(),
                can_restart_application: current_app_bundle().is_some(),
                mode: self.mode,
                profiles: backend
                    .profiles
                    .list()
                    .iter()
                    .map(|p| (p.id.clone(), p.name.clone()))
                    .collect(),
                selected_profile: selected_profile.clone(),
                can_update_profile: backend.engine.is_some()
                    && backend.profiles.list().iter().any(|p| {
                        Some(&p.id) == selected_profile.as_ref()
                            && matches!(p.source, ProfileSource::Remote { .. })
                    }),
                groups: self
                    .proxies
                    .groups
                    .iter()
                    .filter(|g| !self.proxies.proxies.get(&g.name).is_some_and(|p| p.hidden))
                    .cloned()
                    .collect(),
            };
            if *self.menu != menu {
                self.menu = Arc::new(menu);
            }
            self.menu_dirty = false;
        }
        let snapshot = TraySnapshot {
            menu: self.menu.clone(),
            upload_bytes_per_second: backend.last_traffic.as_ref().map_or(0, |t| t.up),
            download_bytes_per_second: backend.last_traffic.as_ref().map_or(0, |t| t.down),
        };
        if self.last_sent.as_ref() != Some(&snapshot) && updates.try_send(snapshot.clone()).is_ok()
        {
            self.last_sent = Some(snapshot);
        }
    }
}

pub(super) fn send_response(server: &IpcServer, response: UiResponse) {
    // Unsolicited tray changes must also decode in older GUIs, which do not know
    // new write commands. They only need the resulting native state.
    let response = match response {
        UiResponse::SystemProxy { result, .. } => UiResponse::SystemProxy {
            request: SystemProxyCommand::GetState,
            result,
        },
        other => other,
    };
    if let Some(primary) = server.primary() {
        let _ = server.send(
            primary,
            ClientMessage::Response(UiResponseEnvelope::realtime(response)),
        );
    }
}

pub(super) fn execute(backend: &mut Backend, server: &IpcServer, command: TrayCommand) {
    let result = execute_inner(backend, server, command);
    if let Err(error) = result {
        backend.log_app("error", format!("托盘操作失败: {}", error.message));
        let _ = crate::platform::MacNotifier::new(ProcessRunner).notify("Verge", &error.message);
    }
}

pub(super) fn execute_inner(
    backend: &mut Backend,
    server: &IpcServer,
    command: TrayCommand,
) -> Result<(), AppError> {
    let request = match command {
        TrayCommand::SetMode(mode) => UiRequest::Runtime(RuntimeCommand::SetMode { mode }),
        TrayCommand::SelectProxy { group, proxy } => {
            UiRequest::Runtime(RuntimeCommand::SelectProxy { group, proxy })
        }
        TrayCommand::SelectProfile(id) => UiRequest::Profile(AppCommand::SelectProfile { id }),
        TrayCommand::UpdateCurrentProfile => UiRequest::Profile(AppCommand::UpdateRemoteProfile {
            id: backend
                .profiles
                .selected()
                .cloned()
                .ok_or_else(|| AppError::new(ErrorCode::NotFound, "no active profile"))?,
        }),
        TrayCommand::RestartCore => UiRequest::Profile(AppCommand::SelectProfile {
            // Selecting the active profile validates then restarts the supervisor,
            // waits for health and restores the previous runtime on failure.
            id: backend
                .profiles
                .selected()
                .cloned()
                .ok_or_else(|| AppError::new(ErrorCode::NotFound, "no active profile"))?,
        }),
        TrayCommand::RestartApplication => UiRequest::Profile(AppCommand::RestartApplication),
        TrayCommand::ToggleSystemProxy => {
            let state = backend.system_proxy.state()?;
            UiRequest::SystemProxy(SystemProxyCommand::SetEnabled {
                enabled: !state.unified_enabled(),
            })
        }

        TrayCommand::OpenDirectory(directory) => {
            let path = backend.config.data_dir.join(match directory {
                TrayDirectory::Data => "",
                TrayDirectory::Profiles => "profiles",
                TrayDirectory::Logs => "logs",
            });
            fs::create_dir_all(&path)
                .map_err(|e| AppError::new(ErrorCode::StorageFailed, e.to_string()))?;
            let status = std::process::Command::new("/usr/bin/open")
                .arg(&path)
                .status()
                .map_err(|e| AppError::new(ErrorCode::PlatformFailed, e.to_string()))?;
            return if status.success() {
                Ok(())
            } else {
                Err(AppError::new(
                    ErrorCode::PlatformFailed,
                    "could not open directory",
                ))
            };
        }
        TrayCommand::ShowMainWindow | TrayCommand::Quit => unreachable!("handled by daemon loop"),
    };
    let refresh_profiles = matches!(request, UiRequest::Profile(_));
    let response = backend.execute(request);
    let error = match &response {
        UiResponse::Profile { result: Err(e), .. }
        | UiResponse::Runtime { result: Err(e), .. }
        | UiResponse::SystemProxy { result: Err(e), .. } => Some(e.clone()),
        _ => None,
    };
    send_response(server, response);
    if let Some(error) = error {
        return Err(error);
    }
    if refresh_profiles {
        for command in [AppCommand::ListProfiles, AppCommand::GetRuntimeSettings] {
            send_response(server, backend.execute(UiRequest::Profile(command)));
        }
        if server.primary().is_some() {
            for command in [
                RuntimeCommand::GetMode,
                RuntimeCommand::ListProxyGroups,
                RuntimeCommand::ListRules,
            ] {
                let mut response = backend.execute(UiRequest::Runtime(command));
                backend.order_proxy_groups(&mut response);
                send_response(server, response);
            }
        }
    }
    Ok(())
}

pub(super) fn changes_menu(request: &UiRequest) -> bool {
    match request {
        UiRequest::SystemProxy(command) => !matches!(command, SystemProxyCommand::GetState),
        UiRequest::Runtime(command) => matches!(
            command,
            RuntimeCommand::SetMode { .. }
                | RuntimeCommand::SelectProxy { .. }
                | RuntimeCommand::SetNetworkSettings { .. }
                | RuntimeCommand::UpdateProvider { .. }
        ),
        UiRequest::Profile(command) => command.risk() != crate::domain::CommandRisk::ReadOnly,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_refresh_cannot_override_completed_write() {
        let (backend, directory) = super::super::tests::test_backend("tray-stale");
        let (server, _) = IpcServer::bind(&PathBuf::from(format!(
            "/tmp/verge-tray-test-{}.sock",
            std::process::id()
        )))
        .unwrap();
        let mut state = TrayState::new();
        state.in_flight = true;
        state.mode = Some(RunMode::Global);
        state.invalidate();
        state.accept(
            RefreshResult {
                generation: 0,
                mode: Some(RunMode::Rule),
                proxies: ProxySnapshot::default(),
                system_proxy: None,
            },
            &server,
        );
        assert_eq!(state.mode, Some(RunMode::Global));
        assert!(!state.in_flight);
        let (tx, rx) = mpsc::sync_channel(1);
        state.publish(&backend, &tx);
        let first = rx.recv().unwrap();
        state.publish(&backend, &tx);
        assert!(
            rx.try_recv().is_err(),
            "unchanged snapshots do not queue native updates"
        );
        assert!(Arc::ptr_eq(&first.menu, &state.menu));
        server.shutdown();
        drop(backend);
        let _ = fs::remove_dir_all(directory);
    }
    #[test]
    fn delay_queries_and_realtime_do_not_trigger_system_proxy_reads() {
        assert!(!changes_menu(&UiRequest::Runtime(
            RuntimeCommand::TestProxyDelay {
                proxy: "A".into(),
                url: "https://example.invalid".into(),
                timeout_ms: 5000
            }
        )));
        assert!(!changes_menu(&UiRequest::Runtime(
            RuntimeCommand::DrainRealtime
        )));
        assert!(changes_menu(&UiRequest::Runtime(RuntimeCommand::SetMode {
            mode: RunMode::Rule
        })));
        assert!(changes_menu(&UiRequest::SystemProxy(
            SystemProxyCommand::Disable
        )));
    }
}
