use std::{
    env, fs,
    net::{SocketAddr, TcpListener},
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::application::app_update::{
    APP_EXECUTABLE_NAME, DittoArchiver, MacCodesign, check_app_update, update_application_bundle,
};
use crate::application::{
    ApplicationLifecycle, CommandBus, MihomoRuntime, PlatformSystemProxy, ProfileCommandHandler,
    ProfileFetcher, ProfileUpdateCoordinator, ReqwestArtifactFetcher, ReqwestProfileFetcher,
    RuntimeCommandHandler, RuntimeCredentials, SupervisorControl, SystemProxyCommandHandler,
    SystemProxyControl, download_mihomo_candidate,
};
use crate::config::{
    FileProfileStore, FileSettingsStore, UpdateScheduler, UpdateTrigger, decrypt_backup,
    write_encrypted_backup,
};
use crate::domain::{
    AppCommand, AppCommandOutput, AppCommandResult, AppError, ApplicationSettings,
    ApplicationSettingsSnapshot, ErrorCode, LogEvent, Profile, RealtimeEvent, RuntimeCommand,
    RuntimeCommandOutput, RuntimeSettings, SystemProxyCommand, SystemProxyCommandResult,
    TrafficEvent,
};
use crate::ipc::{
    ClientMessage, InitialSnapshot, IpcServer, IpcServerEvent, PROTOCOL_VERSION, ServerError,
};
use crate::mihomo::{
    CoreSupervisor, MihomoClient, MihomoConfig, RealtimeOptions, SidecarManifest,
    TcpControllerTransport, install_verified_artifact,
};
use crate::platform::{
    HELPER_SOCKET_PATH, HelperInstallLayout, MacHelperInstaller, bundle_short_version,
    bundled_resources_directory as platform_bundled_resources, current_app_bundle, current_uid,
    discover_bundled_helper,
};
use crate::platform::{
    LoginItemService, MacHelperClient, MacSystemProxy, ProcessRunner, SingleInstance, TrayCommand,
    TrayService, TraySnapshot, daemon_socket_path, default_login_item_service,
    redirect_stderr_to_log,
};
use crate::ui::{UiRequest, UiRequestEnvelope, UiResponse, UiResponseEnvelope};

#[derive(Clone)]
struct BackendConfig {
    data_dir: PathBuf,
    binary: PathBuf,
    manifest: PathBuf,
    controller: SocketAddr,
    secret: String,
    services: Vec<String>,
    recovery_path: PathBuf,
    helper_socket: PathBuf,
}

impl BackendConfig {
    fn internal_socket(&self) -> PathBuf {
        self.data_dir.join("control/mihomo.sock")
    }

    /// The primary runtime and background queries must use the same private controller.
    fn build_runtime(&self) -> Result<MihomoRuntime<TcpControllerTransport>, AppError> {
        let socket = self.internal_socket();
        if socket.as_os_str().len() >= 104 {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Verge data directory is too long for the internal controller socket",
            ));
        }
        let transport = TcpControllerTransport::unix(socket.clone(), Duration::from_secs(2))?;
        let mut runtime = MihomoRuntime::new(
            MihomoClient::new(transport),
            self.controller,
            self.secret.clone(),
            RealtimeOptions::default(),
        )?;
        runtime.set_internal_socket(socket);
        Ok(runtime)
    }

    fn from_env() -> Result<Self, AppError> {
        let controller = match env::var("VERGE_CONTROLLER") {
            Ok(value) => value
                .parse::<SocketAddr>()
                .map_err(|error| AppError::new(ErrorCode::InvalidInput, error.to_string()))?,
            Err(_) => available_controller()?,
        };
        if !controller.ip().is_loopback() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "VERGE_CONTROLLER must use a loopback address",
            ));
        }
        let secret = env::var("VERGE_SECRET").unwrap_or(generate_secret()?);
        let services = env::var("VERGE_NETWORK_SERVICES")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|service| !service.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let data_dir = data_directory()?;
        let bundled_resources = bundled_resources_directory();
        let manifest = env::var_os("VERGE_MIHOMO_MANIFEST")
            .map(PathBuf::from)
            .or_else(|| {
                bundled_resources
                    .as_ref()
                    .map(|resources| resources.join("mihomo-manifest.json"))
                    .filter(|path| path.is_file())
            })
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/mihomo/manifest.json")
            });
        let managed_binary = data_dir.join("bin/mihomo");
        let managed_is_valid = SidecarManifest::load(&manifest)
            .and_then(|manifest| {
                manifest
                    .target(sidecar_target()?)?
                    .verify_executable(&managed_binary)
            })
            .is_ok();
        let binary = env::var_os("VERGE_MIHOMO_BIN")
            .map(PathBuf::from)
            .or_else(|| managed_is_valid.then_some(managed_binary))
            .or_else(|| {
                bundled_resources
                    .as_ref()
                    .map(|resources| resources.join("bin/mihomo"))
                    .filter(|path| path.is_file())
            })
            .unwrap_or_else(|| data_dir.join("bin/mihomo"));
        Ok(Self {
            binary,
            manifest,
            controller,
            secret,
            services,
            recovery_path: data_dir.join("system-proxy-recovery.json"),
            helper_socket: env::var_os("VERGE_HELPER_SOCKET")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(HELPER_SOCKET_PATH)),
            data_dir,
        })
    }
}

pub fn data_directory() -> Result<PathBuf, AppError> {
    env::var_os("VERGE_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join("Library/Application Support/Verge"))
        })
        .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "no Verge data directory"))
}

fn bundled_resources_directory() -> Option<PathBuf> {
    let executable = env::current_exe().ok()?;
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents")
        .then(|| contents.join("Resources"))
}

/// 运行实例版本：读自己 bundle 的 Info.plist；非 .app 运行返回 None。
fn current_app_version() -> Option<String> {
    current_app_bundle().and_then(|bundle| bundle_short_version(&bundle).ok())
}

struct Engine {
    supervisor: CoreSupervisor,
    runtime: MihomoRuntime<TcpControllerTransport>,
}

struct Backend {
    config: BackendConfig,
    profiles: FileProfileStore,
    settings: FileSettingsStore,
    engine: Option<Engine>,
    runtime_error: Option<AppError>,
    system_proxy: PlatformSystemProxy<MacSystemProxy<ProcessRunner>>,
    login_item: Box<dyn LoginItemService + Send>,
    update_scheduler: UpdateScheduler,
    profile_fetcher: ReqwestProfileFetcher,
    artifact_fetcher: ReqwestArtifactFetcher,
    /// 应用自身更新专用下载器（host 白名单含 api.github.com，限额更大）。
    app_fetcher: ReqwestArtifactFetcher,
    /// RestartApplication 已拉起新进程，事件循环收尾后退出本进程。
    restart_requested: bool,
    /// QuitApplication 请求完整退出，事件循环收尾时恢复代理并停止内核。
    quit_requested: bool,
    startup_updates_pending: bool,
    last_update_poll: Instant,
    /// 最近一次流量事件，用于守护进程托盘速度文字的更新。
    last_traffic: Option<TrafficEvent>,
    /// 全局快捷键注册同步请求通道（守护主线程消费；测试中 None 表示未接线）。
    hotkey_sync: Option<Sender<Option<String>>>,
    /// 应用自身运行事件的有界缓冲（引擎状态、IPC、错误等），
    /// 经实时流发给 GUI 日志页显示（与 Mihomo 内核日志并列）。
    app_log_buffer: std::collections::VecDeque<LogEvent>,
}

impl Backend {
    fn new(config: BackendConfig) -> Result<Self, AppError> {
        let mut profiles = FileProfileStore::open_with_keychain(config.data_dir.join("profiles"))?;
        let control_dir = config.data_dir.join("control");
        fs::create_dir_all(&control_dir)
            .map_err(|e| AppError::new(ErrorCode::StorageFailed, e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&control_dir, fs::Permissions::from_mode(0o700))
                .map_err(|e| AppError::new(ErrorCode::StorageFailed, e.to_string()))?;
        }
        profiles.set_internal_socket(config.internal_socket());
        let settings = FileSettingsStore::open(&config.data_dir)?;
        let mut platform = MacSystemProxy::new(ProcessRunner, config.recovery_path.clone());
        let services = if config.services.is_empty() {
            platform.list_network_services()?
        } else {
            config.services.clone()
        };
        let mut system_proxy = PlatformSystemProxy::new(platform, services)?;
        if system_proxy.recovery_pending() {
            system_proxy.recover_pending()?;
        }
        let mut backend = Self {
            config,
            profiles,
            settings,
            engine: None,
            runtime_error: None,
            system_proxy,
            login_item: default_login_item_service(),
            update_scheduler: UpdateScheduler::default(),
            profile_fetcher: ReqwestProfileFetcher::new(Duration::from_secs(20), 8 * 1024 * 1024)?,
            artifact_fetcher: ReqwestArtifactFetcher::new(
                Duration::from_secs(60),
                64 * 1024 * 1024,
                [
                    "github.com".to_owned(),
                    "release-assets.githubusercontent.com".to_owned(),
                    "objects.githubusercontent.com".to_owned(),
                ],
            )?,
            app_fetcher: ReqwestArtifactFetcher::new(
                Duration::from_secs(300),
                512 * 1024 * 1024,
                [
                    "api.github.com".to_owned(),
                    "github.com".to_owned(),
                    "release-assets.githubusercontent.com".to_owned(),
                    "objects.githubusercontent.com".to_owned(),
                ],
            )?,
            restart_requested: false,
            quit_requested: false,
            startup_updates_pending: true,
            last_update_poll: Instant::now(),
            last_traffic: None,
            hotkey_sync: None,
            app_log_buffer: std::collections::VecDeque::new(),
        };
        if backend.profiles.selected().is_some()
            && let Err(error) = backend.start_selected()
        {
            backend.log_app("error", format!("选中配置启动内核失败: {}", error.message));
            backend.runtime_error = Some(error);
        }
        backend.log_app("info", "守护进程后端就绪");
        Ok(backend)
    }

    fn build_engine(&self, profile: &crate::domain::ProfileId) -> Result<Engine, AppError> {
        let config = &self.config;
        let manifest = SidecarManifest::load(&config.manifest)?;
        let target = manifest.target(sidecar_target()?)?;
        target.verify_executable(&config.binary)?;
        let working_dir = config.data_dir.join("mihomo");
        fs::create_dir_all(&working_dir)
            .map_err(|error| AppError::new(ErrorCode::StorageFailed, error.to_string()))?;
        let runtime_config =
            self.profiles
                .materialize_runtime(profile, config.controller, &config.secret)?;
        let mut supervisor = CoreSupervisor::new(
            MihomoConfig {
                binary: config.binary.clone(),
                working_dir,
                controller: config.controller,
                secret: config.secret.clone(),
                max_restarts: 3,
                restart_backoff: Duration::from_secs(1),
                stop_timeout: Duration::from_secs(3),
                log_capacity: 2_000,
            },
            runtime_config,
        )?;
        supervisor.set_internal_socket(config.internal_socket());
        let mut runtime = config.build_runtime()?;
        runtime.set_sensitive_values(self.profiles.list().iter().filter_map(|profile| {
            match &profile.source {
                crate::domain::ProfileSource::Remote { url } => Some(url.clone()),
                crate::domain::ProfileSource::Local => None,
            }
        }));
        Ok(Engine {
            supervisor,
            runtime,
        })
    }

    fn start_selected(&mut self) -> Result<(), AppError> {
        let selected = self.profiles.selected().cloned().ok_or_else(|| {
            AppError::new(ErrorCode::NotFound, "no selected profile to start Mihomo")
        })?;
        self.log_app("info", format!("启动内核 (profile={})", selected.as_str()));
        let mut engine = self.build_engine(&selected)?;
        engine
            .supervisor
            .validate_candidate(&self.profiles.materialize_runtime(
                &selected,
                self.config.controller,
                &self.config.secret,
            )?)?;
        let mut core = SupervisorControl::new(&mut engine.supervisor, Duration::from_secs(5));
        ApplicationLifecycle::new(&mut core, &mut engine.runtime, &mut self.system_proxy)
            .start()
            .map_err(|error| error.cause)?;
        self.engine = Some(engine);
        self.refresh_sensitive_values();
        self.runtime_error = None;
        self.startup_updates_pending = true;
        Ok(())
    }

    fn shutdown(&mut self) {
        if let Some(mut engine) = self.engine.take() {
            let mut core = SupervisorControl::new(&mut engine.supervisor, Duration::from_secs(5));
            let _ =
                ApplicationLifecycle::new(&mut core, &mut engine.runtime, &mut self.system_proxy)
                    .shutdown();
        }
    }

    fn fail_runtime(&mut self, error: AppError) {
        self.shutdown();
        self.runtime_error = Some(error);
    }

    fn poll(&mut self) -> Result<(), AppError> {
        if let Some(engine) = &mut self.engine {
            engine.supervisor.poll(Instant::now())?;
        }
        if self.engine.is_some()
            && (self.last_update_poll.elapsed() >= Duration::from_secs(60)
                || self.startup_updates_pending)
        {
            self.update_due_profiles()?;
            self.last_update_poll = Instant::now();
            self.startup_updates_pending = false;
        }
        Ok(())
    }

    fn update_due_profiles(&mut self) -> Result<(), AppError> {
        let Some(engine) = &mut self.engine else {
            return Ok(());
        };
        let trigger = if self.startup_updates_pending {
            UpdateTrigger::Startup
        } else {
            UpdateTrigger::Scheduled
        };
        let mut core = SupervisorControl::new(&mut engine.supervisor, Duration::from_secs(5));
        let credentials = RuntimeCredentials {
            controller: self.config.controller,
            secret: self.config.secret.clone(),
        };
        let _results = ProfileUpdateCoordinator::new(
            &mut self.profiles,
            &mut core,
            &mut self.update_scheduler,
            &mut self.profile_fetcher,
            Some(credentials),
        )
        .update_due(unix_timestamp()?, trigger)?;
        Ok(())
    }

    fn execute(&mut self, request: UiRequest) -> UiResponse {
        match request {
            UiRequest::Profile(request) => {
                let result = self.execute_profile(request.clone());
                UiResponse::Profile { request, result }
            }
            UiRequest::Runtime(request) => {
                let unavailable = self.runtime_unavailable();
                let result = match &mut self.engine {
                    Some(engine) => {
                        // TUN 开关需要 helper 协调时由 handler 经 HelperControl 完成。
                        let helper = MacHelperClient::new(&self.config.helper_socket);
                        RuntimeCommandHandler::with_helper(&mut engine.runtime, &helper)
                            .execute(request.clone())
                    }
                    None => Err(unavailable),
                };
                if result.is_ok()
                    && let RuntimeCommand::SetNetworkSettings { settings } = &request
                {
                    self.profiles.preserve_runtime_tun(settings.tun_enabled);
                }
                UiResponse::Runtime { request, result }
            }
            UiRequest::SystemProxy(request) => {
                let result =
                    SystemProxyCommandHandler::new(&mut self.system_proxy).execute(request.clone());
                UiResponse::SystemProxy { request, result }
            }
        }
    }

    fn execute_profile(&mut self, command: AppCommand) -> Result<AppCommandResult, AppError> {
        let now = unix_timestamp()?;
        match &command {
            AppCommand::GetCoreNetworkSettings => {
                return Ok(AppCommandResult {
                    output: AppCommandOutput::CoreNetworkSettings(
                        self.profiles.network_settings()?,
                    ),
                    summary: "Network overrides loaded".into(),
                });
            }
            AppCommand::UpdateCoreNetworkSettings { settings } => {
                self.update_core_network(settings.clone())?;
                return Ok(AppCommandResult {
                    output: AppCommandOutput::CoreNetworkSettings(
                        self.profiles.network_settings()?,
                    ),
                    summary: "Network overrides saved and applied".into(),
                });
            }
            AppCommand::GetRuntimeSettings => {
                let selected = self
                    .profiles
                    .selected()
                    .ok_or_else(|| AppError::new(ErrorCode::NotFound, "no active profile"))?;
                return Ok(AppCommandResult {
                    output: AppCommandOutput::RuntimeSettings(crate::domain::RuntimeSettings {
                        system_proxy_services: self.system_proxy.managed_services().to_vec(),
                        system_proxy_endpoint: self.profiles.system_proxy_endpoint(selected)?,
                        system_proxy_socks_endpoint: self
                            .profiles
                            .system_proxy_socks_endpoint(selected)?,
                    }),
                    summary: "Runtime settings loaded".into(),
                });
            }
            AppCommand::GetApplicationSettings => {
                return Ok(AppCommandResult {
                    output: AppCommandOutput::ApplicationSettings(self.settings_snapshot()),
                    summary: "Application settings loaded".into(),
                });
            }
            AppCommand::GetHelperStatus => {
                return Ok(AppCommandResult {
                    output: AppCommandOutput::HelperStatus(self.helper_status()),
                    summary: "Privileged helper status loaded".into(),
                });
            }
            AppCommand::InstallHelper => {
                let status = self.install_helper()?;
                return Ok(AppCommandResult {
                    output: AppCommandOutput::HelperStatus(status),
                    summary: "Privileged helper installed".into(),
                });
            }
            AppCommand::UninstallHelper => {
                let status = self.uninstall_helper()?;
                return Ok(AppCommandResult {
                    output: AppCommandOutput::HelperStatus(status),
                    summary: "Privileged helper uninstalled".into(),
                });
            }
            AppCommand::UpdateApplicationSettings { settings } => {
                self.persist_settings(settings)?;
            }
            AppCommand::ExportApplicationSettings { destination } => {
                let path = self.export_application_settings(destination)?;
                return Ok(AppCommandResult {
                    output: AppCommandOutput::ApplicationSettingsExported { path },
                    summary: "Application settings exported".into(),
                });
            }
            AppCommand::PreviewApplicationSettingsImport { source } => {
                let preview = self.preview_settings_import(source)?;
                return Ok(AppCommandResult {
                    output: AppCommandOutput::ApplicationSettingsImportPreview(preview),
                    summary: "Application settings import preview loaded".into(),
                });
            }
            AppCommand::ImportApplicationSettings { source } => {
                self.import_application_settings(source)?;
                return Ok(AppCommandResult {
                    output: AppCommandOutput::ApplicationSettings(self.settings_snapshot()),
                    summary: "Application settings imported".into(),
                });
            }
            AppCommand::ResetApplicationSettingsScope { scope } => {
                let mut settings = self.settings.get().clone();
                settings.reset_scope(*scope);
                self.persist_settings(&settings)?;
                return Ok(AppCommandResult {
                    output: AppCommandOutput::ApplicationSettings(self.settings_snapshot()),
                    summary: "Application settings scope reset to defaults".into(),
                });
            }
            AppCommand::ExportDiagnostics { destination } => {
                let path = self.export_diagnostics(destination)?;
                return Ok(AppCommandResult {
                    output: AppCommandOutput::DiagnosticsExported { path },
                    summary: "Redacted diagnostics exported".into(),
                });
            }
            AppCommand::ExportEncryptedBackup { passphrase } => {
                let path = self.config.data_dir.join("backups/verge-backup.vgbak");
                let bundle = self.profiles.backup_bundle(self.settings.get().clone())?;
                write_encrypted_backup(&path, &bundle, passphrase)?;
                return Ok(AppCommandResult {
                    output: AppCommandOutput::EncryptedBackupExported {
                        path: path.display().to_string(),
                    },
                    summary: "Encrypted local backup exported".into(),
                });
            }
            AppCommand::RestoreEncryptedBackup { passphrase } => {
                self.restore_encrypted_backup(passphrase)?;
            }
            AppCommand::UpdateMihomo => {
                let version = self.update_mihomo()?;
                return Ok(AppCommandResult {
                    output: AppCommandOutput::MihomoUpdated { version },
                    summary: "Mihomo updated and health checked".into(),
                });
            }
            AppCommand::CheckAppUpdate => {
                let status = self.check_app_update()?;
                return Ok(AppCommandResult {
                    output: AppCommandOutput::AppUpdateStatus(status),
                    summary: "Application update status loaded".into(),
                });
            }
            AppCommand::UpdateApplication => {
                let version = self.update_application()?;
                return Ok(AppCommandResult {
                    output: AppCommandOutput::ApplicationUpdated {
                        version,
                        restart_required: true,
                    },
                    summary: "Application updated; restart to apply".into(),
                });
            }
            AppCommand::RestartApplication => {
                self.request_restart()?;
                return Ok(AppCommandResult {
                    output: AppCommandOutput::None,
                    summary: "Application restart initiated".into(),
                });
            }
            AppCommand::QuitApplication => {
                self.log_app("info", "收到退出请求，正在清理系统状态");
                self.quit_requested = true;
                return Ok(AppCommandResult {
                    output: AppCommandOutput::None,
                    summary: "Application quit initiated".into(),
                });
            }
            AppCommand::ListProfiles => {
                return Ok(AppCommandResult {
                    output: AppCommandOutput::Profiles {
                        profiles: self.profiles.list().to_vec(),
                        selected: self.profiles.selected().cloned(),
                    },
                    summary: "Profiles loaded".into(),
                });
            }
            AppCommand::GetProfileYaml { id } => {
                return Ok(AppCommandResult {
                    output: AppCommandOutput::ProfileYaml {
                        id: id.clone(),
                        yaml: self.profiles.yaml(id)?,
                    },
                    summary: "Profile YAML loaded".into(),
                });
            }
            AppCommand::GetMergeConfig => {
                return Ok(AppCommandResult {
                    output: AppCommandOutput::MergeConfigYaml {
                        yaml: self.profiles.merge_yaml()?,
                    },
                    summary: "Merge config loaded".into(),
                });
            }
            AppCommand::GetMergedProfileYaml { id } => {
                return Ok(AppCommandResult {
                    output: AppCommandOutput::MergedProfileYaml {
                        id: id.clone(),
                        yaml: self.profiles.merged_yaml(id)?,
                    },
                    summary: "Merged profile YAML loaded".into(),
                });
            }
            AppCommand::ImportProfile {
                id,
                name,
                yaml,
                source,
                update_policy,
            } => {
                let profile = Profile::new(
                    id.clone(),
                    name,
                    source.clone(),
                    update_policy.clone(),
                    now,
                    None,
                )?;
                self.profiles.import(profile, yaml)?;
            }
            AppCommand::ImportRemoteProfile {
                id,
                name,
                url,
                update_policy,
                user_agent,
            } => {
                let yaml = self.profile_fetcher.fetch(url, user_agent.as_deref())?;
                let profile = Profile::new(
                    id.clone(),
                    name,
                    crate::domain::ProfileSource::Remote { url: url.clone() },
                    update_policy.clone(),
                    now,
                    user_agent.clone(),
                )?;
                self.profiles.import(profile, &yaml)?;
            }
            AppCommand::DeleteProfile { id } if self.profiles.selected() != Some(id) => {
                self.profiles.delete(id)?;
            }
            AppCommand::DeleteProfile { .. } => {
                return Err(AppError::new(
                    ErrorCode::Conflict,
                    "select another profile before deleting the active profile",
                ));
            }
            AppCommand::SetProfileUpdatePolicy { id, update_policy } => {
                self.profiles
                    .set_update_policy(id, update_policy.clone(), now)?;
            }
            AppCommand::SelectProfile { id } if self.engine.is_none() => {
                self.profiles.yaml(id)?;
                let previous = self.profiles.selected().cloned();
                self.profiles.select(id)?;
                if let Err(error) = self.start_selected() {
                    match previous {
                        Some(previous) => {
                            let _ = self.profiles.select(&previous);
                        }
                        None => {
                            let _ = self.profiles.clear_selection();
                        }
                    }
                    return Err(error);
                }
            }
            AppCommand::UpdateProfileYaml { .. } if self.engine.is_none() => {
                return Err(self.runtime_unavailable());
            }
            AppCommand::UpdateMergeConfig { .. } if self.engine.is_none() => {
                return Err(self.runtime_unavailable());
            }
            AppCommand::UpdateRemoteProfile { .. } if self.engine.is_none() => {
                return Err(self.runtime_unavailable());
            }
            AppCommand::UpdateRemoteProfile { id } => {
                let engine = self.engine.as_mut().expect("engine was checked above");
                let mut core =
                    SupervisorControl::new(&mut engine.supervisor, Duration::from_secs(5));
                let credentials = RuntimeCredentials {
                    controller: self.config.controller,
                    secret: self.config.secret.clone(),
                };
                ProfileUpdateCoordinator::new(
                    &mut self.profiles,
                    &mut core,
                    &mut self.update_scheduler,
                    &mut self.profile_fetcher,
                    Some(credentials),
                )
                .update_manual(id, now)
                .map_err(|error| error.cause)?;
            }
            AppCommand::SelectProfile { .. }
            | AppCommand::UpdateProfileYaml { .. }
            | AppCommand::UpdateMergeConfig { .. } => {
                let engine = self.engine.as_mut().expect("engine was checked above");
                let mut core =
                    SupervisorControl::new(&mut engine.supervisor, Duration::from_secs(5));
                let credentials = RuntimeCredentials {
                    controller: self.config.controller,
                    secret: self.config.secret.clone(),
                };
                ProfileCommandHandler::new(&mut self.profiles, &mut core, Some(credentials))
                    .execute(command, now)
                    .map_err(|error| error.cause)?;
            }
        }
        self.refresh_sensitive_values();
        Ok(AppCommandResult {
            output: AppCommandOutput::None,
            summary: "Profile command completed".into(),
        })
    }

    fn update_core_network(
        &mut self,
        settings: crate::domain::CoreNetworkSettings,
    ) -> Result<(), AppError> {
        settings.validate()?;
        let Some(engine) = &mut self.engine else {
            // Persist even without a subscription; the next start uses the same overrides.
            return self.profiles.set_network_override(Some(settings));
        };
        use crate::application::RuntimeControl as _;
        let current = engine.runtime.network_settings()?;
        self.profiles.preserve_runtime_tun(current.tun_enabled);
        let selected = self.profiles.selected().cloned();
        let old_http = selected
            .as_ref()
            .and_then(|id| self.profiles.system_proxy_endpoint(id).ok());
        let old_socks = selected
            .as_ref()
            .and_then(|id| self.profiles.system_proxy_socks_endpoint(id).ok().flatten());
        let services = self.system_proxy.managed_services().to_vec();
        let mut proxy = MacSystemProxy::new(ProcessRunner, self.config.recovery_path.clone());
        let mut core = SupervisorControl::new(&mut engine.supervisor, Duration::from_secs(5));
        let credentials = RuntimeCredentials {
            controller: self.config.controller,
            secret: self.config.secret.clone(),
        };
        crate::application::ConfigCommandHandler::with_runtime_credentials(
            &mut self.profiles,
            &mut core,
            credentials,
        )
        .update_network_settings(settings, |profiles| {
            if let (Some(id), Some(old)) = (&selected, &old_http) {
                let new = profiles.system_proxy_endpoint(id)?;
                if &new != old || old_socks.as_ref().is_some_and(|s| s != &new) {
                    proxy.remap_endpoints(&services, old, old_socks.as_ref(), &new)?;
                }
            }
            Ok(())
        })?;
        self.refresh_sensitive_values();
        Ok(())
    }

    /// 诊断导出的文本脱敏:controller secret、订阅地址、用户目录、认证头。
    /// 在写入 JSON 之前作用于各自由文本字段,保持结构与既有脱敏规则不变。
    fn redact_text(&self, text: &str) -> String {
        let mut redacted = text.replace(&self.config.secret, "[REDACTED_SECRET]");
        let mut sensitive = self
            .profiles
            .list()
            .iter()
            .filter_map(|profile| match &profile.source {
                crate::domain::ProfileSource::Remote { url } => Some(url.clone()),
                crate::domain::ProfileSource::Local => None,
            })
            .collect::<Vec<_>>();
        if let Some(settings) = self.profiles.network_override() {
            sensitive.push(settings.external_controller.secret);
        }
        if let Ok(home) = env::var("HOME") {
            sensitive.push(home);
        }
        sensitive.retain(|value| !value.is_empty());
        sensitive.sort_by_key(|value| std::cmp::Reverse(value.len()));
        sensitive.dedup();
        for value in sensitive {
            redacted = redacted.replace(&value, "[REDACTED]");
        }
        for marker in ["Authorization: Bearer ", "Proxy-Authorization: "] {
            let mut search_from = 0;
            while let Some(relative) = redacted
                .get(search_from..)
                .and_then(|rest| rest.find(marker))
            {
                let value_start = search_from + relative + marker.len();
                let value_end = redacted[value_start..]
                    .find(char::is_whitespace)
                    .map_or(redacted.len(), |offset| value_start + offset);
                redacted.replace_range(value_start..value_end, "[REDACTED]");
                search_from = value_start + "[REDACTED]".len();
            }
        }
        redacted
    }

    fn helper_status(&self) -> crate::domain::HelperStatus {
        MacHelperClient::new(&self.config.helper_socket).status()
    }

    /// 安装/修复特权 helper：只在非就绪状态时执行；非 .app 运行给可读错误。
    /// 安装本身即“清残留 + 重装”，覆盖 NotInstalled 与 Incompatible 两种修复路径。
    fn install_helper(&mut self) -> Result<crate::domain::HelperStatus, AppError> {
        if let status @ crate::domain::HelperStatus::Ready { .. } = self.helper_status() {
            return Ok(status);
        }
        let resources = platform_bundled_resources().ok_or_else(|| {
            AppError::new(
                ErrorCode::PlatformFailed,
                "helper installation requires a packaged Verge.app; run apps/verge/scripts/build-macos-app.sh first",
            )
        })?;
        let bundle = discover_bundled_helper(&resources)?;
        let mut installer = MacHelperInstaller::new(
            ProcessRunner,
            HelperInstallLayout::system(),
            self.config.data_dir.join("helper-install"),
            current_uid(),
        )?;
        installer.install(&bundle)?;
        Ok(self.helper_status())
    }

    /// 卸载特权 helper：移除 LaunchDaemon 与二进制（提权脚本在 installer 内）。
    fn uninstall_helper(&mut self) -> Result<crate::domain::HelperStatus, AppError> {
        let mut installer = MacHelperInstaller::new(
            ProcessRunner,
            HelperInstallLayout::system(),
            self.config.data_dir.join("helper-install"),
            current_uid(),
        )?;
        installer.uninstall()?;
        Ok(self.helper_status())
    }

    /// 导出明文设置文件(带版本号,不含任何 Keychain 密钥或订阅凭据)。
    fn export_application_settings(&self, destination: &str) -> Result<String, AppError> {
        let destination = PathBuf::from(destination);
        if destination.as_os_str().is_empty() || !destination.is_absolute() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "settings export destination must be an absolute path",
            ));
        }
        let bytes = crate::config::export_settings_json(self.settings.get())?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| AppError::new(ErrorCode::StorageFailed, error.to_string()))?;
        }
        fs::write(&destination, bytes)
            .map_err(|error| AppError::new(ErrorCode::StorageFailed, error.to_string()))?;
        Ok(destination.display().to_string())
    }

    fn read_settings_import(&self, source: &str) -> Result<Vec<u8>, AppError> {
        let source = PathBuf::from(source);
        if source.as_os_str().is_empty() || !source.is_absolute() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "settings import source must be an absolute path",
            ));
        }
        fs::read(&source)
            .map_err(|error| AppError::new(ErrorCode::StorageFailed, error.to_string()))
    }

    /// 导入前预览:解析校验导入文件,并给出与当前设置的逐字段差异。
    fn preview_settings_import(
        &self,
        source: &str,
    ) -> Result<crate::domain::SettingsImportPreview, AppError> {
        let bytes = self.read_settings_import(source)?;
        crate::config::settings_import_preview(self.settings.get(), &bytes)
    }

    /// 应用导入:重新解析校验(与 UpdateApplicationSettings 相同的校验),再持久化。
    fn import_application_settings(
        &mut self,
        source: &str,
    ) -> Result<crate::domain::ApplicationSettings, AppError> {
        let bytes = self.read_settings_import(source)?;
        let settings = crate::config::parse_settings_import(&bytes)?;
        self.persist_settings(&settings)?;
        Ok(settings)
    }

    /// 设置落盘前先同步系统侧副作用（登录启动）。副作用失败则整体失败、
    /// 设置不落盘，保持持久化设置与系统状态一致。
    fn persist_settings(&mut self, settings: &ApplicationSettings) -> Result<(), AppError> {
        if settings.launch_at_login != self.settings.get().launch_at_login {
            self.login_item.set_enabled(settings.launch_at_login)?;
        }
        let previous_hotkey = self.settings.get().global_hotkey.clone();
        self.settings.update(settings.clone())?;
        // 全局快捷键由守护进程持有：设置变更落盘后请求主线程重新注册。
        // 注册失败不影响设置生效，由守护侧 OS 通知告知用户（守护没有 toast 能力）。
        if settings.global_hotkey != previous_hotkey {
            self.queue_hotkey_sync(settings.global_hotkey.as_deref());
        }
        Ok(())
    }

    /// 请求守护主线程同步全局快捷键注册（None = 禁用）。未接线时（测试）静默跳过。
    fn queue_hotkey_sync(&self, desired: Option<&str>) {
        if let Some(tx) = &self.hotkey_sync {
            let _ = tx.send(desired.map(str::to_owned));
        }
    }

    fn export_diagnostics(&mut self, destination: &str) -> Result<String, AppError> {
        let destination = PathBuf::from(destination);
        if destination.as_os_str().is_empty() || !destination.is_absolute() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "diagnostic destination must be an absolute path",
            ));
        }
        let profiles = self
            .profiles
            .list()
            .iter()
            .map(|profile| {
                serde_json::json!({
                    "id": profile.id.as_str(),
                    "name": self.redact_text(&profile.name),
                    "source": match profile.source {
                        crate::domain::ProfileSource::Local => "local",
                        crate::domain::ProfileSource::Remote { .. } => "remote_redacted",
                    },
                    "updated_at": profile.updated_at,
                    "next_update_at": profile.next_update_at,
                    "consecutive_failures": profile.consecutive_failures,
                    "last_error": profile.last_error.as_deref().map(|error| self.redact_text(error)),
                })
            })
            .collect::<Vec<_>>();
        // helper/系统代理/内核版本都是尽力而为的快照,失败只记录脱敏后的错误文本。
        let helper = self.helper_status();
        let helper_status = match serde_json::to_value(&helper) {
            Ok(mut value) => {
                if let Some(message) = value.get_mut("message").and_then(|m| m.as_str()) {
                    let redacted = self.redact_text(message);
                    value["message"] = serde_json::Value::String(redacted);
                }
                value
            }
            Err(_) => serde_json::json!({ "state": "unknown" }),
        };
        let system_proxy = match self.system_proxy.state() {
            Ok(state) => serde_json::to_value(&state)
                .unwrap_or_else(|_| serde_json::json!({ "error": "serialization failed" })),
            Err(error) => serde_json::json!({ "error": self.redact_text(&error.message) }),
        };
        let mihomo_version = SidecarManifest::load(&self.config.manifest)
            .map(|manifest| manifest.version)
            .ok();
        let merge = self.profiles.merge();
        let report = serde_json::json!({
            "schema_version": 2,
            "generated_at": unix_timestamp()?,
            "app_version": env!("CARGO_PKG_VERSION"),
            "platform": std::env::consts::OS,
            "architecture": std::env::consts::ARCH,
            "core_running": self.engine.is_some(),
            "runtime_error": self.runtime_error.as_ref().map(|error| self.redact_text(&error.message)),
            "helper_status": helper_status,
            "system_proxy": system_proxy,
            "mihomo_version": mihomo_version,
            "merge_config": {
                "enabled": !merge.rules.is_empty(),
                "rule_count": merge.rules.len(),
            },
            "selected_profile": self.profiles.selected().map(|id| id.as_str()),
            "profiles": profiles,
            "settings": self.settings.get(),
        });
        let bytes = serde_json::to_vec_pretty(&report)
            .map_err(|error| AppError::new(ErrorCode::StorageFailed, error.to_string()))?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| AppError::new(ErrorCode::StorageFailed, error.to_string()))?;
        }
        fs::write(&destination, bytes)
            .map_err(|error| AppError::new(ErrorCode::StorageFailed, error.to_string()))?;
        Ok(destination.display().to_string())
    }

    fn restore_encrypted_backup(&mut self, passphrase: &str) -> Result<(), AppError> {
        let previous_hotkey = self.settings.get().global_hotkey.clone();
        let path = self.config.data_dir.join("backups/verge-backup.vgbak");
        let bytes = fs::read(&path)
            .map_err(|error| AppError::new(ErrorCode::StorageFailed, error.to_string()))?;
        let restored = decrypt_backup(&bytes, passphrase)?;
        let previous = self.profiles.backup_bundle(self.settings.get().clone())?;
        self.shutdown();
        let restored_settings = match self.profiles.restore_backup(restored) {
            Ok(settings) => settings,
            Err(error) => {
                let _ = self.start_selected();
                return Err(error);
            }
        };
        if let Err(error) = self.settings.update(restored_settings) {
            let previous_settings = self.profiles.restore_backup(previous)?;
            let _ = self.settings.update(previous_settings);
            let _ = self.start_selected();
            return Err(error);
        }
        if self.profiles.selected().is_some()
            && let Err(error) = self.start_selected()
        {
            self.shutdown();
            let previous_settings = self.profiles.restore_backup(previous)?;
            self.settings.update(previous_settings)?;
            let recovery = self.start_selected().err();
            return Err(match recovery {
                Some(recovery) => AppError::new(
                    ErrorCode::CoreUnavailable,
                    format!(
                        "restored backup failed to start: {}; previous state recovery failed: {}",
                        error.message, recovery.message
                    ),
                ),
                None => error,
            });
        }
        self.refresh_sensitive_values();
        // 备份恢复绕过了 persist_settings，全局快捷键变更在这里补齐同步。
        let restored_hotkey = self.settings.get().global_hotkey.clone();
        if restored_hotkey != previous_hotkey {
            self.queue_hotkey_sync(restored_hotkey.as_deref());
        }
        Ok(())
    }

    fn update_mihomo(&mut self) -> Result<String, AppError> {
        let manifest = SidecarManifest::load(&self.config.manifest)?;
        let target = manifest.target(sidecar_target()?)?.clone();
        let staging = self.config.data_dir.join("updates/mihomo");
        let candidate = download_mihomo_candidate(&mut self.artifact_fetcher, &target, &staging)?;
        let managed_binary = self.config.data_dir.join("bin/mihomo");
        let old_binary = self.config.binary.clone();

        self.shutdown();
        let install =
            match install_verified_artifact(&candidate, &managed_binary, &target.executable_sha256)
            {
                Ok(install) => install,
                Err(error) => {
                    let _ = self.start_selected();
                    return Err(error);
                }
            };
        self.config.binary = managed_binary;
        if let Err(error) = self.start_selected() {
            self.shutdown();
            self.config.binary = old_binary;
            let rollback_error = install.rollback().err();
            let restart_error = self.start_selected().err();
            return Err(match rollback_error.or(restart_error) {
                Some(recovery) => AppError::new(
                    ErrorCode::CoreUnavailable,
                    format!(
                        "updated Mihomo failed health check: {}; recovery failed: {}",
                        error.message, recovery.message
                    ),
                ),
                None => error,
            });
        }
        install.commit()?;
        let _ = fs::remove_file(candidate);
        Ok(manifest.version)
    }

    /// 应用设置快照：附带运行实例版本（非 .app 运行为 None，UI 据此提示）。
    fn settings_snapshot(&self) -> ApplicationSettingsSnapshot {
        ApplicationSettingsSnapshot {
            settings: self.settings.get().clone(),
            data_directory: self.config.data_dir.display().to_string(),
            app_version: current_app_version(),
        }
    }

    /// 检查应用自身更新（只读；非 .app 运行时当前版本为 None，不会误报可更新）。
    fn check_app_update(&mut self) -> Result<crate::domain::AppUpdateStatus, AppError> {
        check_app_update(&mut self.app_fetcher, current_app_version().as_deref())
    }

    /// 应用自身更新事务：非 .app 运行直接给可读错误，其余见
    /// `crate::application::app_update` 的模块注释（校验、签名、替换与回滚策略）。
    fn update_application(&mut self) -> Result<String, AppError> {
        let bundle = current_app_bundle().ok_or_else(|| {
            AppError::new(
                ErrorCode::PlatformFailed,
                "application update requires a packaged Verge.app; build one with apps/verge/scripts/build-macos-app.sh first",
            )
        })?;
        self.log_app("info", "开始下载并校验应用更新…");
        let mut archiver = DittoArchiver::new(ProcessRunner);
        let mut signer = MacCodesign;
        let mut runner = ProcessRunner;
        let staging = self.config.data_dir.join("updates/app");
        let outcome = update_application_bundle(
            &mut self.app_fetcher,
            &mut archiver,
            &mut signer,
            &mut runner,
            &bundle,
            &staging,
            current_uid(),
        )?;
        self.log_app(
            "info",
            format!("应用更新 v{} 已就位，重启后生效", outcome.version),
        );
        Ok(outcome.version)
    }

    /// 重启编排（双进程）：先拉起（可能已替换的）bundle 里的 GUI 进程并标记退出；
    /// 事件循环收尾（停内核、恢复代理、关 IPC）后退出本进程，新进程按既有
    /// connect-or-spawn 逻辑接管守护角色。
    fn request_restart(&mut self) -> Result<(), AppError> {
        let bundle = current_app_bundle().ok_or_else(|| {
            AppError::new(
                ErrorCode::PlatformFailed,
                "application restart requires a packaged Verge.app",
            )
        })?;
        let executable = bundle.join("Contents/MacOS").join(APP_EXECUTABLE_NAME);
        if !executable.is_file() {
            return Err(AppError::new(
                ErrorCode::PlatformFailed,
                format!("bundle executable is missing: {}", executable.display()),
            ));
        }
        std::process::Command::new(&executable)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|error| AppError::new(ErrorCode::PlatformFailed, error.to_string()))?;
        self.log_app("info", "新实例已拉起，守护进程退出完成重启");
        self.restart_requested = true;
        Ok(())
    }

    fn refresh_sensitive_values(&mut self) {
        let mut values = self
            .profiles
            .list()
            .iter()
            .filter_map(|profile| match &profile.source {
                crate::domain::ProfileSource::Remote { url } => Some(url.clone()),
                crate::domain::ProfileSource::Local => None,
            })
            .collect::<Vec<_>>();
        if let Some(settings) = self.profiles.network_override() {
            values.push(settings.external_controller.secret);
        }
        if let Some(engine) = &mut self.engine {
            engine.runtime.set_sensitive_values(values);
        }
    }

    fn runtime_unavailable(&self) -> AppError {
        self.runtime_error.clone().unwrap_or_else(|| {
            AppError::new(
                ErrorCode::CoreUnavailable,
                "select a profile to start Mihomo",
            )
        })
    }

    fn order_proxy_groups(&self, response: &mut UiResponse) {
        let UiResponse::Runtime {
            result: Ok(result), ..
        } = response
        else {
            return;
        };
        let RuntimeCommandOutput::ProxyGroups(snapshot) = &mut result.output else {
            return;
        };
        if let Ok(order) = self.profiles.proxy_group_order() {
            let indices: std::collections::HashMap<_, _> = order
                .iter()
                .enumerate()
                .map(|(i, name)| (name.as_str(), i))
                .collect();
            snapshot.groups.sort_by_key(|group| {
                indices
                    .get(group.name.as_str())
                    .copied()
                    .unwrap_or(usize::MAX)
            });
        }
    }

    fn query_runtime(&self) -> Result<MihomoRuntime<TcpControllerTransport>, AppError> {
        if self.engine.is_none() {
            return Err(self.runtime_unavailable());
        }
        self.config.build_runtime()
    }

    /// 拉取实时事件缓冲（守护进程事件循环在 RealtimeTick 时调用）。
    /// 返回的事件已经过脱敏；同时记录最近流量供托盘速度文字使用。
    /// 记录一条应用运行事件：写 stderr（已重定向到日志文件），并推入有界缓冲
    /// 以便经实时流显示在 GUI 日志页。
    fn log_app(&mut self, level: &'static str, message: impl std::fmt::Display) {
        let line = format!("[{level}] {message}");
        eprintln!("[verge] {line}");
        self.app_log_buffer.push_back(LogEvent {
            level: level.to_owned(),
            payload: line,
        });
        const APP_LOG_CAP: usize = 500;
        while self.app_log_buffer.len() > APP_LOG_CAP {
            self.app_log_buffer.pop_front();
        }
    }

    fn drain_realtime_events(&mut self) -> Vec<RealtimeEvent> {
        // 先把应用自身事件带出去（不依赖引擎）。
        let app_logs: Vec<RealtimeEvent> = self
            .app_log_buffer
            .drain(..)
            .map(RealtimeEvent::Log)
            .collect();
        let Some(engine) = self.engine.as_mut() else {
            return app_logs;
        };
        let result = RuntimeCommandHandler::new(&mut engine.runtime)
            .execute(RuntimeCommand::DrainRealtime)
            .ok();
        match result {
            Some(result) => match result.output {
                RuntimeCommandOutput::Realtime(events) => {
                    for event in &events {
                        if let RealtimeEvent::Traffic(traffic) = event {
                            self.last_traffic = Some(traffic.clone());
                        }
                    }
                    let mut merged = app_logs;
                    merged.extend(events);
                    merged
                }
                _ => app_logs,
            },
            None => app_logs,
        }
    }

    /// 组装 IPC 握手快照：一次拉全可重建的领域态。
    fn initial_snapshot(&self) -> InitialSnapshot {
        InitialSnapshot {
            profiles: self.profiles.list().to_vec(),
            selected_profile: self.profiles.selected().cloned(),
            application_settings: ApplicationSettingsSnapshot {
                settings: self.settings.get().clone(),
                data_directory: self.config.data_dir.display().to_string(),
                app_version: current_app_version(),
            },
            runtime_settings: self.runtime_settings_snapshot(),
        }
    }

    /// 当前选中配置对应的运行时设置（无选中配置时为 None）。
    fn runtime_settings_snapshot(&self) -> Option<RuntimeSettings> {
        let selected = self.profiles.selected()?;
        Some(RuntimeSettings {
            system_proxy_services: self.system_proxy.managed_services().to_vec(),
            system_proxy_endpoint: self.profiles.system_proxy_endpoint(selected).ok()?,
            system_proxy_socks_endpoint: self
                .profiles
                .system_proxy_socks_endpoint(selected)
                .ok()?,
        })
    }

    /// 系统代理当前是否开启（任一受管网络服务的 HTTP 代理开启即视为开启）。
    fn system_proxy_enabled(&mut self) -> bool {
        self.system_proxy
            .state()
            .is_ok_and(|state| state.services.iter().any(|service| service.web.enabled))
    }

    /// 托盘"切换系统代理"：当前开则关，关则按选中配置的端点开启。
    /// 守护进程直接执行，不再经过 GUI 进程。
    fn toggle_system_proxy(&mut self) -> Result<SystemProxyCommandResult, AppError> {
        let state = self.system_proxy.state()?;
        if state.services.iter().any(|service| service.web.enabled) {
            return SystemProxyCommandHandler::new(&mut self.system_proxy)
                .execute(SystemProxyCommand::Disable);
        }
        let selected = self
            .profiles
            .selected()
            .cloned()
            .ok_or_else(|| AppError::new(ErrorCode::NotFound, "no active profile"))?;
        let endpoint = self.profiles.system_proxy_endpoint(&selected)?;
        let services = self.system_proxy.managed_services().to_vec();
        SystemProxyCommandHandler::new(&mut self.system_proxy)
            .execute(SystemProxyCommand::Enable { services, endpoint })
    }
}

/// 守护进程入口。`main()` 以 `--daemon` 参数分叉到此处。
///
/// 线程模型：
/// - 主线程：无窗口 NSApplication 事件循环（accessory，无 Dock 图标），
///   通过 GCD 主队列定时器消费托盘状态更新（事件驱动，非轮询）；
/// - backend 线程：消费统一事件队列（IPC 请求、worker 结果、托盘命令、
///   实时聚合 tick、健康/调度 tick），阻塞等待，零忙等。
pub fn run_daemon() {
    let config = match BackendConfig::from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!(
                "Verge daemon failed to resolve configuration: {}",
                error.message
            );
            std::process::exit(1);
        }
    };
    // 双击启动没有终端：先把 stderr 重定向到数据目录日志，daemon 的运行日志落盘可查。
    if let Err(error) = redirect_stderr_to_log(&config.data_dir) {
        eprintln!("failed to redirect daemon stderr to log: {error}");
    }
    eprintln!(
        "[verge] daemon starting, data_dir={}",
        config.data_dir.display()
    );
    let socket = daemon_socket_path(&config.data_dir);
    let _single_instance = match SingleInstance::acquire(&config.data_dir) {
        Ok(instance) => instance,
        Err(error) => {
            eprintln!("Verge daemon is already running: {}", error.message);
            std::process::exit(1);
        }
    };
    let (server, server_events) = match IpcServer::bind(&socket) {
        Ok(pair) => pair,
        Err(ServerError::AlreadyRunning) => {
            eprintln!("Verge daemon is already running (IPC socket in use)");
            std::process::exit(1);
        }
        Err(ServerError::Io(error)) => {
            eprintln!("Verge daemon failed to bind IPC socket: {error}");
            std::process::exit(1);
        }
    };
    server.spawn_accept();
    initialize_daemon_appkit();
    let (tray_tx, tray_rx) = mpsc::channel::<TrayCommand>();
    let tray = match TrayService::new(move |command| {
        let _ = tray_tx.send(command);
    }) {
        Ok(tray) => tray,
        Err(error) => {
            eprintln!("Verge daemon failed to initialize tray: {}", error.message);
            std::process::exit(1);
        }
    };
    let (tray_updates_tx, tray_updates_rx) = mpsc::channel::<TraySnapshot>();
    // 全局快捷键：backend 线程发注册同步请求，主线程消费（GlobalHotKeyManager
    // 必须在主线程创建和调用）；backend 收尾完成后经 shutdown channel 请求
    // 主线程终止 AppKit 循环，守护进程真正退出。
    let (hotkey_sync_tx, hotkey_sync_rx) = mpsc::channel::<Option<String>>();
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
    let backend_thread = std::thread::spawn(move || {
        run_daemon_backend(
            config,
            server,
            server_events,
            tray_rx,
            tray_updates_tx,
            hotkey_sync_tx,
            shutdown_tx,
        );
    });
    run_daemon_appkit(tray, tray_updates_rx, hotkey_sync_rx, shutdown_rx);
    // 主线程返回只发生在 AppKit 终止之后；backend 已完成清理。
    let _ = backend_thread.join();
}

/// Initialize AppKit before constructing any tray/menu objects.
///
/// `tray-icon` reaches into CoreGraphics while building the status item. A daemon
/// spawned directly from the executable has not gone through LaunchServices, so
/// creating the tray before `NSApplication` aborts in `CGSConnectionByID`.
#[cfg(target_os = "macos")]
fn initialize_daemon_appkit() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let marker = MainThreadMarker::new().expect("daemon main must run on the main thread");
    let app = NSApplication::sharedApplication(marker);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

#[cfg(not(target_os = "macos"))]
fn initialize_daemon_appkit() {}

/// backend 事件循环：所有事件源投递到统一队列，单线程阻塞消费。
fn run_daemon_backend(
    config: BackendConfig,
    server: Arc<IpcServer>,
    server_events: Receiver<IpcServerEvent>,
    tray_rx: Receiver<TrayCommand>,
    tray_updates: Sender<TraySnapshot>,
    hotkey_sync: Sender<Option<String>>,
    shutdown: Sender<()>,
) {
    let mut backend = match Backend::new(config) {
        Ok(backend) => backend,
        Err(error) => {
            eprintln!("Verge daemon backend failed to start: {}", error.message);
            std::process::exit(1);
        }
    };
    backend.hotkey_sync = Some(hotkey_sync);
    // 启动即按已持久化设置同步一次全局快捷键注册。
    backend.queue_hotkey_sync(backend.settings.get().global_hotkey.as_deref());
    // 启动即推一次托盘快照：菜单文案按已持久化语言贴上。
    let mut tray_language = backend.settings.get().language.clone();
    let _ = tray_updates.send(TraySnapshot {
        language: tray_language.clone(),
        ..TraySnapshot::default()
    });
    let (event_tx, event_rx) = mpsc::channel::<DaemonEvent>();

    // 全局快捷键按下事件转发线程（global-hotkey 全局 channel 的消费端）。
    #[cfg(target_os = "macos")]
    {
        let hotkey_tx = event_tx.clone();
        crate::platform::spawn_hotkey_listener(move || {
            let _ = hotkey_tx.send(DaemonEvent::HotkeyPressed);
        });
    }

    // IPC 事件转发线程。
    let ipc_tx = event_tx.clone();
    std::thread::spawn(move || {
        while let Ok(event) = server_events.recv() {
            if ipc_tx.send(DaemonEvent::Ipc(event)).is_err() {
                break;
            }
        }
    });
    // 托盘命令转发线程（muda 全局 channel 的消费端）。
    let tray_forward_tx = event_tx.clone();
    std::thread::spawn(move || {
        while let Ok(command) = tray_rx.recv() {
            if tray_forward_tx.send(DaemonEvent::Tray(command)).is_err() {
                break;
            }
        }
    });
    // 实时聚合 tick：100ms 一次，drain 实时缓冲并批量转发。
    let realtime_tx = event_tx.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(100));
            if realtime_tx.send(DaemonEvent::RealtimeTick).is_err() {
                break;
            }
        }
    });
    // 健康/调度 tick：1s 一次（supervisor 轮询 + profile 更新调度）。
    let poll_tx = event_tx.clone();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(1));
            if poll_tx.send(DaemonEvent::PollTick).is_err() {
                break;
            }
        }
    });

    let mut command_bus = CommandBus::default();
    let mut isolated_jobs = 0_usize;
    let mut quit = false;
    while !quit {
        let event = match event_rx.recv() {
            Ok(event) => event,
            Err(_) => break,
        };
        match event {
            DaemonEvent::Ipc(IpcServerEvent::Connected {
                conn_id,
                protocol_version,
                ..
            }) => {
                backend.log_app(
                    "info",
                    format!("GUI 连接 (conn={conn_id}, protocol={protocol_version})"),
                );
                handle_daemon_connected(&server, &backend, conn_id, protocol_version);
            }
            DaemonEvent::Ipc(IpcServerEvent::Request { conn_id, envelope }) => {
                handle_daemon_request(
                    &mut backend,
                    &server,
                    &mut command_bus,
                    &event_tx,
                    conn_id,
                    envelope,
                    &mut isolated_jobs,
                );
            }
            DaemonEvent::Ipc(IpcServerEvent::Disconnected { conn_id }) => {
                server.release_primary(conn_id);
                server.clear_subscriptions(conn_id);
            }
            DaemonEvent::WorkerDone {
                conn_id,
                envelope,
                mut response,
            } => {
                backend.order_proxy_groups(&mut response);
                isolated_jobs = isolated_jobs.saturating_sub(1);
                let risk = envelope.request.risk();
                let last_in_operation = envelope.operation_index + 1 == envelope.operation_len;
                command_bus.complete(
                    envelope.operation_id,
                    risk,
                    response_succeeded(&response),
                    last_in_operation,
                );
                let _ = server.send(
                    conn_id,
                    ClientMessage::Response(UiResponseEnvelope::for_request(&envelope, *response)),
                );
            }
            // 全局快捷键按下 = 托盘"Show Verge"：唤醒主 GUI 或拉起 GUI 进程。
            DaemonEvent::Tray(TrayCommand::ShowMainWindow) | DaemonEvent::HotkeyPressed => {
                show_main_window(&server);
            }
            DaemonEvent::Tray(TrayCommand::HideMainWindow) => {
                if let Some(primary) = server.primary() {
                    let _ = server.send(primary, ClientMessage::HideWindow);
                }
            }
            DaemonEvent::Tray(TrayCommand::ToggleSystemProxy) => {
                if let Ok(result) = backend.toggle_system_proxy() {
                    let _ = tray_updates.send(TraySnapshot {
                        system_proxy_enabled: result
                            .state
                            .services
                            .iter()
                            .any(|service| service.web.enabled),
                        upload_bytes_per_second: backend
                            .last_traffic
                            .as_ref()
                            .map_or(0, |traffic| traffic.up),
                        download_bytes_per_second: backend
                            .last_traffic
                            .as_ref()
                            .map_or(0, |traffic| traffic.down),
                        language: backend.settings.get().language.clone(),
                    });
                }
            }
            DaemonEvent::Tray(TrayCommand::Quit) => quit = true,
            DaemonEvent::RealtimeTick => {
                let events = backend.drain_realtime_events();
                if !events.is_empty() {
                    server.broadcast_realtime(&events);
                    if let Some(traffic) = backend.last_traffic.clone() {
                        let _ = tray_updates.send(TraySnapshot {
                            system_proxy_enabled: backend.system_proxy_enabled(),
                            upload_bytes_per_second: traffic.up,
                            download_bytes_per_second: traffic.down,
                            language: backend.settings.get().language.clone(),
                        });
                    }
                }
            }
            DaemonEvent::PollTick => {
                if let Some(error) = backend.poll().err() {
                    backend.log_app("error", format!("内核轮询失败: {}", error.message));
                    backend.fail_runtime(error);
                }
            }
        }
        // 语言变化（UpdateSettings / 导入 / 恢复备份等任一路径）时推一次托盘快照，
        // 主线程据此重贴菜单文案；不需要专门的设置事件通道。
        let language = backend.settings.get().language.clone();
        if language != tray_language {
            tray_language = language;
            let _ = tray_updates.send(TraySnapshot {
                system_proxy_enabled: backend.system_proxy_enabled(),
                upload_bytes_per_second: backend
                    .last_traffic
                    .as_ref()
                    .map_or(0, |traffic| traffic.up),
                download_bytes_per_second: backend
                    .last_traffic
                    .as_ref()
                    .map_or(0, |traffic| traffic.down),
                language: tray_language.clone(),
            });
        }
        // 重启或退出请求已受理：随本次事件一起收尾退出。
        if backend.restart_requested || backend.quit_requested {
            quit = true;
        }
    }
    backend.shutdown();
    server.shutdown();
    if backend.restart_requested {
        // 新 GUI 进程已在 request_restart 时拉起，会在连接失败后自建守护；
        // 直接结束进程，AppKit 主线程（托盘）随进程终止。
        std::process::exit(0);
    }
    // 清理完成：请求主线程终止 AppKit 事件循环，守护进程随 main 返回真正退出。
    let _ = shutdown.send(());
}

/// 托盘"显示 Verge"与守护侧全局快捷键共用：有主 GUI 则激活其窗口，
/// 否则拉起新的 GUI 进程（它会经 IPC 连接本守护）。
fn show_main_window(server: &Arc<IpcServer>) {
    if let Some(primary) = server.primary() {
        let _ = server.send(primary, ClientMessage::ActivateWindow);
    } else {
        let _ = spawn_gui_process();
    }
}

/// 新连接完成握手后的处理：版本校验、主实例裁定、初始快照。
fn handle_daemon_connected(
    server: &Arc<IpcServer>,
    backend: &Backend,
    conn_id: u64,
    protocol_version: u32,
) {
    if protocol_version != PROTOCOL_VERSION {
        let _ = server.send(
            conn_id,
            ClientMessage::Closed {
                reason: format!(
                    "protocol version mismatch: daemon={PROTOCOL_VERSION}, client={protocol_version}"
                ),
            },
        );
        server.close(conn_id);
        return;
    }
    if !server.try_claim_primary(conn_id) {
        if let Some(primary) = server.primary() {
            let _ = server.send(primary, ClientMessage::ActivateWindow);
        }
        let _ = server.send(conn_id, ClientMessage::Duplicate);
        server.close(conn_id);
        return;
    }
    let initial = backend.initial_snapshot();
    let _ = server.send(
        conn_id,
        ClientMessage::Welcome {
            protocol_version: PROTOCOL_VERSION,
            initial,
        },
    );
}

/// 处理一条 IPC 请求：隔离执行或同步执行，响应回给发起连接。
fn handle_daemon_request(
    backend: &mut Backend,
    server: &Arc<IpcServer>,
    command_bus: &mut CommandBus,
    event_tx: &Sender<DaemonEvent>,
    conn_id: u64,
    envelope: UiRequestEnvelope,
    isolated_jobs: &mut usize,
) {
    let request = envelope.request.clone();
    let risk = request.risk();
    let last_in_operation = envelope.operation_index + 1 == envelope.operation_len;
    if is_isolated_runtime_request(&request) {
        if *isolated_jobs >= 8 {
            let response = failed_response(
                request,
                AppError::new(
                    ErrorCode::Conflict,
                    "too many Mihomo queries are already running",
                ),
            );
            command_bus.complete(envelope.operation_id, risk, false, last_in_operation);
            let _ = server.send(
                conn_id,
                ClientMessage::Response(UiResponseEnvelope::for_request(&envelope, response)),
            );
            return;
        }
        let runtime = match command_bus
            .authorize(envelope.operation_id, envelope.context, risk)
            .and_then(|()| backend.query_runtime())
        {
            Ok(runtime) => runtime,
            Err(error) => {
                let response = failed_response(request, error);
                command_bus.complete(envelope.operation_id, risk, false, last_in_operation);
                let _ = server.send(
                    conn_id,
                    ClientMessage::Response(UiResponseEnvelope::for_request(&envelope, response)),
                );
                return;
            }
        };
        *isolated_jobs += 1;
        let worker_tx = event_tx.clone();
        std::thread::spawn(move || {
            let response = execute_isolated_runtime(request, runtime);
            let _ = worker_tx.send(DaemonEvent::WorkerDone {
                conn_id,
                envelope,
                response: Box::new(response),
            });
        });
        return;
    }
    let response = match command_bus.authorize(envelope.operation_id, envelope.context, risk) {
        Ok(()) => {
            let response = backend.execute(request);
            // 实时订阅由 StartRealtime/StopRealtime 命令驱动，同步维护订阅表。
            if let UiResponse::Runtime {
                request: RuntimeCommand::StartRealtime { topics },
                result: Ok(_),
            } = &response
            {
                server.set_subscriptions(conn_id, topics.clone());
            }
            if let UiResponse::Runtime {
                request: RuntimeCommand::StopRealtime,
                result: Ok(_),
            } = &response
            {
                server.clear_subscriptions(conn_id);
            }
            response
        }
        Err(error) => failed_response(request, error),
    };
    command_bus.complete(
        envelope.operation_id,
        risk,
        response_succeeded(&response),
        last_in_operation,
    );
    let _ = server.send(
        conn_id,
        ClientMessage::Response(UiResponseEnvelope::for_request(&envelope, response)),
    );
}

/// 守护进程主线程：无窗口 AppKit 事件循环 + 托盘状态刷新 + 全局快捷键注册。
///
/// TrayService 与 HotkeyRegistration 都通过 `MainThreadBound` 绑定主线程
/// （`GlobalHotKeyManager` 同样要求主线程创建和调用）；backend 线程的请求
/// 经 channel 交给本线程旁的消费线程，再 `get_on_main` 派发回主线程执行。
/// 消费线程阻塞 `recv`，事件驱动、零轮询。快捷键注册失败没有 toast 能力，
/// 用 OS 通知（osascript）告知用户。
#[cfg(target_os = "macos")]
fn run_daemon_appkit(
    tray: TrayService,
    tray_updates: Receiver<TraySnapshot>,
    hotkey_sync: Receiver<Option<String>>,
    shutdown: Receiver<()>,
) {
    use crate::platform::{GlobalHotKeyBackend, HotkeyRegistration, MacNotifier};
    use dispatch2::MainThreadBound;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let marker = MainThreadMarker::new().expect("daemon main must run on the main thread");
    let app = NSApplication::sharedApplication(marker);
    // spawn 出来的守护进程不走 LaunchServices，激活策略必须在代码里设置。
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    let bound = Arc::new(MainThreadBound::new(tray, marker));
    std::thread::spawn(move || {
        while let Ok(snapshot) = tray_updates.recv() {
            bound.get_on_main(|tray| {
                let _ = tray.update(&snapshot);
            });
        }
    });
    match GlobalHotKeyBackend::new() {
        Ok(backend) => {
            // MainThreadBound 只提供 &T 访问；注册变更多路复用互斥锁保护。
            let registration = MainThreadBound::new(
                std::sync::Mutex::new(HotkeyRegistration::new(backend)),
                marker,
            );
            std::thread::spawn(move || {
                let mut notifier = MacNotifier::new(ProcessRunner);
                while let Ok(desired) = hotkey_sync.recv() {
                    let result = registration.get_on_main(|registration| {
                        registration
                            .lock()
                            .expect("hotkey registration mutex poisoned")
                            .sync(desired.as_deref())
                    });
                    if let Err(error) = result {
                        eprintln!("[verge] [error] 全局快捷键同步失败: {}", error.message);
                        let _ = notifier.notify("Verge", &error.message);
                    }
                }
            });
        }
        Err(error) => {
            eprintln!(
                "[verge] [error] global hotkey is unavailable: {}",
                error.message
            );
        }
    }
    // backend 收尾完成后的退出请求：terminate 结束 app.run()，进程随 main 返回退出。
    let app_for_shutdown = MainThreadBound::new(app.clone(), marker);
    std::thread::spawn(move || {
        if shutdown.recv().is_ok() {
            app_for_shutdown.get_on_main(|app| app.terminate(None));
        }
    });
    app.run();
}

/// 非 macOS 无托盘型守护。
#[cfg(not(target_os = "macos"))]
fn run_daemon_appkit(
    _tray: TrayService,
    _tray_updates: Receiver<TraySnapshot>,
    _hotkey_sync: Receiver<Option<String>>,
    _shutdown: Receiver<()>,
) {
    unreachable!("daemon mode is only supported on macOS")
}

/// 拉起 GUI 进程（不带 `--daemon` 参数）。GUI 进程会连接本守护的 socket。
fn spawn_gui_process() -> std::io::Result<()> {
    let executable = std::env::current_exe()?;
    std::process::Command::new(executable).spawn()?;
    Ok(())
}

/// 守护进程事件队列的统一事件。
enum DaemonEvent {
    Ipc(IpcServerEvent),
    WorkerDone {
        conn_id: u64,
        envelope: UiRequestEnvelope,
        response: Box<UiResponse>,
    },
    Tray(TrayCommand),
    /// 全局快捷键按下（守护进程持有注册，GUI 不在跑时也能呼出主窗口）。
    HotkeyPressed,
    RealtimeTick,
    PollTick,
}

fn is_isolated_runtime_request(request: &UiRequest) -> bool {
    matches!(
        request,
        UiRequest::Runtime(
            RuntimeCommand::GetMode
                | RuntimeCommand::ListProxyGroups
                | RuntimeCommand::ListRules
                | RuntimeCommand::ListProviders
                | RuntimeCommand::GetNetworkSettings
                | RuntimeCommand::TestProxyDelay { .. }
        )
    )
}

fn execute_isolated_runtime(
    request: UiRequest,
    mut runtime: MihomoRuntime<TcpControllerTransport>,
) -> UiResponse {
    let UiRequest::Runtime(command) = request else {
        unreachable!("only runtime requests are sent to the runtime worker")
    };
    let result = RuntimeCommandHandler::new(&mut runtime).execute(command.clone());
    UiResponse::Runtime {
        request: command,
        result,
    }
}

fn response_succeeded(response: &UiResponse) -> bool {
    match response {
        UiResponse::Profile { result, .. } => result.is_ok(),
        UiResponse::Runtime { result, .. } => result.is_ok(),
        UiResponse::SystemProxy { result, .. } => result.is_ok(),
        UiResponse::Realtime(_) => true,
    }
}

fn available_controller() -> Result<SocketAddr, AppError> {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map_err(|error| AppError::new(ErrorCode::PlatformFailed, error.to_string()))
}

fn generate_secret() -> Result<String, AppError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| AppError::new(ErrorCode::PlatformFailed, error.to_string()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn sidecar_target() -> Result<&'static str, AppError> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Ok("aarch64-apple-darwin")
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        Err(AppError::new(
            ErrorCode::CoreUnavailable,
            "this build has no pinned Mihomo sidecar target",
        ))
    }
}

fn unix_timestamp() -> Result<i64, AppError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| AppError::new(ErrorCode::PlatformFailed, error.to_string()))?
        .as_secs();
    i64::try_from(seconds)
        .map_err(|_| AppError::new(ErrorCode::PlatformFailed, "system time overflowed"))
}

fn failed_response(request: UiRequest, error: AppError) -> UiResponse {
    match request {
        UiRequest::Profile(request) => UiResponse::Profile {
            request,
            result: Err(error),
        },
        UiRequest::Runtime(request) => UiResponse::Runtime {
            request,
            result: Err(error),
        },
        UiRequest::SystemProxy(request) => UiResponse::SystemProxy {
            request,
            result: Err(error),
        },
    }
}

#[cfg(test)]
mod runtime_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ProfileId, ProfileSource, UpdatePolicy};

    #[test]
    fn controller_reads_and_delay_are_isolated_but_state_writes_are_not() {
        assert!(is_isolated_runtime_request(&UiRequest::Runtime(
            RuntimeCommand::ListProxyGroups,
        )));
        assert!(is_isolated_runtime_request(&UiRequest::Runtime(
            RuntimeCommand::TestProxyDelay {
                proxy: "node".into(),
                url: "https://example.com".into(),
                timeout_ms: 1_000,
            },
        )));
        assert!(!is_isolated_runtime_request(&UiRequest::Runtime(
            RuntimeCommand::SetMode {
                mode: crate::domain::RunMode::Rule,
            },
        )));
    }

    #[test]
    fn generated_controller_is_loopback_and_available() {
        let controller = available_controller().unwrap();
        assert!(controller.ip().is_loopback());
        assert_ne!(controller.port(), 0);
    }

    #[test]
    fn generated_secret_is_nonempty_hex_without_control_characters() {
        let secret = generate_secret().unwrap();
        assert_eq!(secret.len(), 64);
        assert!(secret.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn backend_without_selected_profile_still_accepts_first_import() {
        let data_dir = std::env::temp_dir().join(format!(
            "verge-gpui-first-import-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&data_dir).unwrap();
        let config = BackendConfig {
            binary: data_dir.join("missing-mihomo"),
            manifest: data_dir.join("missing-manifest.json"),
            controller: available_controller().unwrap(),
            secret: generate_secret().unwrap(),
            services: vec!["Wi-Fi".into()],
            recovery_path: data_dir.join("recovery.json"),
            helper_socket: data_dir.join("helper.sock"),
            data_dir: data_dir.clone(),
        };
        let mut backend = Backend::new(config).unwrap();
        let id = ProfileId::parse("first").unwrap();

        backend
            .execute_profile(AppCommand::ImportProfile {
                id: id.clone(),
                name: "First".into(),
                yaml: "mode: rule\n".into(),
                source: ProfileSource::Local,
                update_policy: UpdatePolicy::Manual,
            })
            .unwrap();
        let listed = backend.execute_profile(AppCommand::ListProfiles).unwrap();
        assert!(matches!(
            listed.output,
            AppCommandOutput::Profiles { profiles, selected: None }
                if profiles.len() == 1 && profiles[0].id == id
        ));
        let _ = fs::remove_dir_all(data_dir);
    }

    #[derive(Default)]
    struct FakeLoginItemState {
        enabled: bool,
        calls: Vec<bool>,
        fail: bool,
    }

    struct FakeLoginItem {
        state: std::sync::Arc<std::sync::Mutex<FakeLoginItemState>>,
    }

    impl LoginItemService for FakeLoginItem {
        fn status(&mut self) -> Result<bool, AppError> {
            Ok(self.state.lock().unwrap().enabled)
        }

        fn set_enabled(&mut self, enabled: bool) -> Result<(), AppError> {
            let mut state = self.state.lock().unwrap();
            state.calls.push(enabled);
            if state.fail {
                return Err(AppError::new(
                    ErrorCode::PlatformFailed,
                    "launch at login requires a bundled .app",
                ));
            }
            state.enabled = enabled;
            Ok(())
        }
    }

    #[test]
    fn network_settings_save_without_a_selected_profile() {
        let (mut backend, dir) = test_backend("network-settings");
        let settings = crate::domain::CoreNetworkSettings {
            mixed_port: 19097,
            ..Default::default()
        };
        backend
            .execute_profile(AppCommand::UpdateCoreNetworkSettings {
                settings: settings.clone(),
            })
            .unwrap();
        let result = backend
            .execute_profile(AppCommand::GetCoreNetworkSettings)
            .unwrap();
        assert_eq!(
            result.output,
            AppCommandOutput::CoreNetworkSettings(settings)
        );
        assert!(dir.join("profiles/network-settings.yaml").is_file());
        drop(backend);
        let _ = fs::remove_dir_all(dir);
    }

    fn test_backend(name: &str) -> (Backend, PathBuf) {
        let data_dir = std::env::temp_dir().join(format!(
            "verge-gpui-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&data_dir).unwrap();
        let config = BackendConfig {
            binary: data_dir.join("missing-mihomo"),
            manifest: data_dir.join("missing-manifest.json"),
            controller: available_controller().unwrap(),
            secret: generate_secret().unwrap(),
            services: vec!["Wi-Fi".into()],
            recovery_path: data_dir.join("recovery.json"),
            helper_socket: data_dir.join("helper.sock"),
            data_dir: data_dir.clone(),
        };
        (Backend::new(config).unwrap(), data_dir)
    }

    fn current_settings(backend: &mut Backend) -> ApplicationSettings {
        let result = backend
            .execute_profile(AppCommand::GetApplicationSettings)
            .unwrap();
        let AppCommandOutput::ApplicationSettings(snapshot) = result.output else {
            panic!("expected application settings snapshot");
        };
        snapshot.settings
    }

    #[test]
    fn launch_at_login_setting_syncs_system_state_before_persisting() {
        let (mut backend, data_dir) = test_backend("launch-at-login");
        let state = std::sync::Arc::new(std::sync::Mutex::new(FakeLoginItemState::default()));
        backend.login_item = Box::new(FakeLoginItem {
            state: state.clone(),
        });

        // 值未变化:不触碰系统状态。
        backend
            .execute_profile(AppCommand::UpdateApplicationSettings {
                settings: ApplicationSettings::default(),
            })
            .unwrap();
        assert!(state.lock().unwrap().calls.is_empty());

        // 打开:先注册登录项,再落盘。
        backend
            .execute_profile(AppCommand::UpdateApplicationSettings {
                settings: ApplicationSettings {
                    launch_at_login: true,
                    ..ApplicationSettings::default()
                },
            })
            .unwrap();
        assert_eq!(state.lock().unwrap().calls, [true]);
        assert!(current_settings(&mut backend).launch_at_login);

        // 系统侧失败:命令报错且设置不落盘,保持两边一致。
        state.lock().unwrap().fail = true;
        let error = backend
            .execute_profile(AppCommand::UpdateApplicationSettings {
                settings: ApplicationSettings {
                    launch_at_login: false,
                    ..ApplicationSettings::default()
                },
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::PlatformFailed);
        assert!(current_settings(&mut backend).launch_at_login);

        // 恢复默认（System 作用域）也会同步关闭登录项。
        state.lock().unwrap().fail = false;
        backend
            .execute_profile(AppCommand::ResetApplicationSettingsScope {
                scope: crate::domain::SettingsScope::System,
            })
            .unwrap();
        assert_eq!(state.lock().unwrap().calls, [true, false, false]);
        assert!(!current_settings(&mut backend).launch_at_login);
        let _ = fs::remove_dir_all(data_dir);
    }

    #[test]
    fn global_hotkey_setting_round_trips_and_is_validated() {
        let (mut backend, data_dir) = test_backend("global-hotkey");
        let error = backend
            .execute_profile(AppCommand::UpdateApplicationSettings {
                settings: ApplicationSettings {
                    global_hotkey: Some("V".into()),
                    ..ApplicationSettings::default()
                },
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert_eq!(current_settings(&mut backend).global_hotkey, None);

        backend
            .execute_profile(AppCommand::UpdateApplicationSettings {
                settings: ApplicationSettings {
                    global_hotkey: Some("CmdOrCtrl+Shift+V".into()),
                    ..ApplicationSettings::default()
                },
            })
            .unwrap();
        assert_eq!(
            current_settings(&mut backend).global_hotkey,
            Some("CmdOrCtrl+Shift+V".into())
        );
        let _ = fs::remove_dir_all(data_dir);
    }

    /// 设置变更（含 scope 重置、备份恢复路径）时守护侧发出热键注册同步请求；
    /// 值未变化不发请求，避免主线程无谓往返。
    #[test]
    fn global_hotkey_change_queues_daemon_registration_sync() {
        let (mut backend, data_dir) = test_backend("hotkey-sync");
        let (tx, rx) = mpsc::channel();
        backend.hotkey_sync = Some(tx);

        // 值未变化：不发同步请求。
        backend
            .execute_profile(AppCommand::UpdateApplicationSettings {
                settings: ApplicationSettings::default(),
            })
            .unwrap();
        assert!(rx.try_recv().is_err());

        // 设置快捷键：同步请求带上原始设置值（规范化在注册侧做）。
        backend
            .execute_profile(AppCommand::UpdateApplicationSettings {
                settings: ApplicationSettings {
                    global_hotkey: Some("CmdOrCtrl+Shift+V".into()),
                    ..ApplicationSettings::default()
                },
            })
            .unwrap();
        assert_eq!(rx.try_recv().unwrap(), Some("CmdOrCtrl+Shift+V".to_owned()));

        // System scope 重置清掉快捷键：同步 None 表示禁用。
        backend
            .execute_profile(AppCommand::ResetApplicationSettingsScope {
                scope: crate::domain::SettingsScope::System,
            })
            .unwrap();
        assert_eq!(rx.try_recv().unwrap(), None);
        assert!(rx.try_recv().is_err());
        let _ = fs::remove_dir_all(data_dir);
    }

    /// 未接线（hotkey_sync = None）时设置变更不得失败——同步请求静默跳过。
    #[test]
    fn global_hotkey_change_without_sync_channel_still_persists() {
        let (mut backend, data_dir) = test_backend("hotkey-sync-unwired");
        backend
            .execute_profile(AppCommand::UpdateApplicationSettings {
                settings: ApplicationSettings {
                    global_hotkey: Some("CmdOrCtrl+Shift+V".into()),
                    ..ApplicationSettings::default()
                },
            })
            .unwrap();
        assert_eq!(
            current_settings(&mut backend).global_hotkey,
            Some("CmdOrCtrl+Shift+V".into())
        );
        let _ = fs::remove_dir_all(data_dir);
    }

    #[test]
    fn diagnostic_export_omits_subscription_urls_secrets_and_yaml() {
        let data_dir = std::env::temp_dir().join(format!(
            "verge-gpui-diagnostics-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&data_dir).unwrap();
        let config = BackendConfig {
            binary: data_dir.join("missing-mihomo"),
            manifest: data_dir.join("missing-manifest.json"),
            controller: available_controller().unwrap(),
            secret: "controller-secret-must-not-leak".into(),
            services: vec!["Wi-Fi".into()],
            recovery_path: data_dir.join("recovery.json"),
            helper_socket: data_dir.join("helper.sock"),
            data_dir: data_dir.clone(),
        };
        let mut backend = Backend::new(config).unwrap();
        backend
            .execute_profile(AppCommand::ImportProfile {
                id: ProfileId::parse("remote").unwrap(),
                name: "Remote".into(),
                yaml: "mixed-port: 7890\nsecret: yaml-secret-must-not-leak\n".into(),
                source: ProfileSource::Remote {
                    url: "https://user:token@example.com/subscription".into(),
                },
                update_policy: UpdatePolicy::Manual,
            })
            .unwrap();
        let destination = data_dir.join("diagnostics.json");
        backend
            .execute_profile(AppCommand::ExportDiagnostics {
                destination: destination.display().to_string(),
            })
            .unwrap();
        let exported = fs::read_to_string(destination).unwrap();
        assert!(exported.contains("remote_redacted"));
        assert!(!exported.contains("subscription"));
        assert!(!exported.contains("controller-secret-must-not-leak"));
        assert!(!exported.contains("yaml-secret-must-not-leak"));
        let backup = backend
            .execute_profile(AppCommand::ExportEncryptedBackup {
                passphrase: "correct horse battery staple".into(),
            })
            .unwrap();
        let AppCommandOutput::EncryptedBackupExported { path } = backup.output else {
            panic!("expected encrypted backup path");
        };
        let encrypted = fs::read_to_string(path).unwrap();
        assert!(!encrypted.contains("subscription"));
        assert!(!encrypted.contains("yaml-secret-must-not-leak"));
        let _ = fs::remove_dir_all(data_dir);
    }

    #[test]
    fn settings_transfer_and_scope_reset_go_through_the_same_validation() {
        let data_dir = std::env::temp_dir().join(format!(
            "verge-gpui-settings-transfer-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&data_dir).unwrap();
        let config = BackendConfig {
            binary: data_dir.join("missing-mihomo"),
            manifest: data_dir.join("missing-manifest.json"),
            controller: available_controller().unwrap(),
            secret: generate_secret().unwrap(),
            services: vec!["Wi-Fi".into()],
            recovery_path: data_dir.join("recovery.json"),
            helper_socket: data_dir.join("helper.sock"),
            data_dir: data_dir.clone(),
        };
        let mut backend = Backend::new(config).unwrap();
        // 先导入一个配置,验证 scope 重置不会顺带删除 Profile。
        backend
            .execute_profile(AppCommand::ImportProfile {
                id: ProfileId::parse("keep").unwrap(),
                name: "Keep".into(),
                yaml: "mode: rule\n".into(),
                source: ProfileSource::Local,
                update_policy: UpdatePolicy::Manual,
            })
            .unwrap();

        let export_path = data_dir.join("settings-export.json");
        let exported = backend
            .execute_profile(AppCommand::ExportApplicationSettings {
                destination: export_path.display().to_string(),
            })
            .unwrap();
        assert!(matches!(
            exported.output,
            AppCommandOutput::ApplicationSettingsExported { .. }
        ));
        let text = fs::read_to_string(&export_path).unwrap();
        assert!(!text.to_lowercase().contains("secret"));
        assert!(!text.to_lowercase().contains("keychain"));

        // 另一台机器带来的设置文件:主题/语言不同。
        let incoming = crate::domain::ApplicationSettings {
            theme: crate::domain::ThemePreference::Dark,
            language: "zh-CN".into(),
            log_limit: 500,
            ..crate::domain::ApplicationSettings::default()
        };
        let import_path = data_dir.join("settings-import.json");
        fs::write(
            &import_path,
            crate::config::export_settings_json(&incoming).unwrap(),
        )
        .unwrap();

        let preview = backend
            .execute_profile(AppCommand::PreviewApplicationSettingsImport {
                source: import_path.display().to_string(),
            })
            .unwrap();
        let AppCommandOutput::ApplicationSettingsImportPreview(preview) = preview.output else {
            panic!("expected settings import preview");
        };
        assert_eq!(preview.settings, incoming);
        assert_eq!(preview.changes.len(), 2);
        // 预览不落盘。
        assert_eq!(
            backend.settings.get(),
            &crate::domain::ApplicationSettings::default()
        );

        let applied = backend
            .execute_profile(AppCommand::ImportApplicationSettings {
                source: import_path.display().to_string(),
            })
            .unwrap();
        assert!(matches!(
            applied.output,
            AppCommandOutput::ApplicationSettings(ref snapshot)
                if snapshot.settings == incoming
        ));
        assert_eq!(backend.settings.get(), &incoming);

        // 版本不符的导入被拒绝,且不改动现有设置。
        fs::write(&import_path, br#"{"version": 99, "settings": {}}"#).unwrap();
        let rejected = backend
            .execute_profile(AppCommand::ImportApplicationSettings {
                source: import_path.display().to_string(),
            })
            .unwrap_err();
        assert_eq!(rejected.code, ErrorCode::ValidationFailed);
        assert_eq!(backend.settings.get(), &incoming);

        // scope 重置只动该作用域:外观重置后语言和主题回默认,log_limit 保持。
        let mut custom = incoming.clone();
        custom.log_limit = 1_000;
        backend
            .execute_profile(AppCommand::UpdateApplicationSettings { settings: custom })
            .unwrap();
        let reset = backend
            .execute_profile(AppCommand::ResetApplicationSettingsScope {
                scope: crate::domain::SettingsScope::Appearance,
            })
            .unwrap();
        assert!(matches!(
            reset.output,
            AppCommandOutput::ApplicationSettings(ref snapshot)
                if snapshot.settings.theme == crate::domain::ThemePreference::System
                    && snapshot.settings.language == "en"
                    && snapshot.settings.log_limit == 1_000
        ));
        let listed = backend.execute_profile(AppCommand::ListProfiles).unwrap();
        assert!(matches!(
            listed.output,
            AppCommandOutput::Profiles { profiles, .. } if profiles.len() == 1
        ));
        let _ = fs::remove_dir_all(data_dir);
    }

    #[test]
    fn diagnostic_export_includes_full_snapshot_and_keeps_redaction() {
        let data_dir = std::env::temp_dir().join(format!(
            "verge-gpui-diagnostics-full-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&data_dir).unwrap();
        let config = BackendConfig {
            binary: data_dir.join("missing-mihomo"),
            manifest: data_dir.join("missing-manifest.json"),
            controller: available_controller().unwrap(),
            secret: "controller-secret-must-not-leak".into(),
            services: vec!["Wi-Fi".into()],
            recovery_path: data_dir.join("recovery.json"),
            helper_socket: data_dir.join("helper.sock"),
            data_dir: data_dir.clone(),
        };
        let mut backend = Backend::new(config).unwrap();
        let id = ProfileId::parse("remote").unwrap();
        backend
            .execute_profile(AppCommand::ImportProfile {
                id: id.clone(),
                name: "Remote".into(),
                yaml: "mixed-port: 7890\nsecret: yaml-secret-must-not-leak\n".into(),
                source: ProfileSource::Remote {
                    url: "https://user:token@example.com/subscription".into(),
                },
                update_policy: UpdatePolicy::Interval { seconds: 300 },
            })
            .unwrap();
        backend
            .profiles
            .set_merge_yaml("rules:\n  - key: mode\n    op: override\n    value: global\n")
            .unwrap();
        let home = env::var("HOME").unwrap();
        backend
            .profiles
            .mark_update_failed(
                &id,
                2_000,
                &AppError::new(
                    ErrorCode::CoreUnavailable,
                    format!(
                        "fetch https://user:token@example.com/subscription failed: Authorization: Bearer abc123 at {home}/Library"
                    ),
                ),
            )
            .unwrap();

        let destination = data_dir.join("diagnostics.json");
        backend
            .execute_profile(AppCommand::ExportDiagnostics {
                destination: destination.display().to_string(),
            })
            .unwrap();
        let exported = fs::read_to_string(&destination).unwrap();
        let report: serde_json::Value = serde_json::from_str(&exported).unwrap();
        assert_eq!(report["schema_version"], 2);
        assert!(report.get("helper_status").is_some());
        assert!(report.get("system_proxy").is_some());
        assert_eq!(report["merge_config"]["enabled"], true);
        assert_eq!(report["merge_config"]["rule_count"], 1);
        assert!(report.get("runtime_error").is_some());
        assert!(
            report["profiles"][0]["last_error"]
                .as_str()
                .unwrap()
                .contains("[REDACTED]")
        );
        // 脱敏回归:secret、订阅 URL、认证头、用户目录、YAML secret 都不能出现。
        for forbidden in [
            "controller-secret-must-not-leak",
            "yaml-secret-must-not-leak",
            "user:token",
            "example.com/subscription",
            "abc123",
            home.as_str(),
        ] {
            assert!(
                !exported.contains(forbidden),
                "diagnostics leaked {forbidden}"
            );
        }
        let _ = fs::remove_dir_all(data_dir);
    }

    /// 非 .app 运行（测试进程）：应用更新/重启在触碰任何系统资源前给可读错误。
    #[test]
    fn app_update_and_restart_outside_a_packaged_app_give_readable_errors() {
        let (mut backend, data_dir) = test_backend("app-update-unpackaged");

        let error = backend
            .execute_profile(AppCommand::UpdateApplication)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::PlatformFailed);
        assert!(error.message.contains("packaged Verge.app"));

        let error = backend
            .execute_profile(AppCommand::RestartApplication)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::PlatformFailed);
        assert!(!backend.restart_requested);
        let _ = fs::remove_dir_all(data_dir);
    }

    #[test]
    fn quit_application_requests_a_clean_daemon_shutdown() {
        let (mut backend, data_dir) = test_backend("quit-application");

        let result = backend
            .execute_profile(AppCommand::QuitApplication)
            .unwrap();

        assert!(backend.quit_requested);
        assert_eq!(result.output, AppCommandOutput::None);
        assert_eq!(result.summary, "Application quit initiated");
        let _ = fs::remove_dir_all(data_dir);
    }

    #[test]
    fn shutdown_without_engine_is_clean() {
        let data_dir = std::env::temp_dir().join(format!(
            "verge-gpui-shutdown-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&data_dir).unwrap();
        let config = BackendConfig {
            binary: data_dir.join("missing-mihomo"),
            manifest: data_dir.join("missing-manifest.json"),
            controller: available_controller().unwrap(),
            secret: generate_secret().unwrap(),
            services: vec!["Wi-Fi".into()],
            recovery_path: data_dir.join("recovery.json"),
            helper_socket: data_dir.join("helper.sock"),
            data_dir: data_dir.clone(),
        };
        let mut backend = Backend::new(config).unwrap();
        // 无引擎、无系统代理改动时，shutdown 必须无副作用地完成。
        backend.shutdown();
        let _ = fs::remove_dir_all(data_dir);
    }

    #[test]
    fn install_helper_outside_a_packaged_app_gives_a_readable_error() {
        let (mut backend, data_dir) = test_backend("helper-install-unpackaged");
        // unix socket 地址有长度上限，用短的不存在路径模拟“未安装”。
        backend.config.helper_socket =
            PathBuf::from(format!("/tmp/verge-helper-missing-{}", std::process::id()));

        // 测试进程不在 .app 内：必须在发起任何提权动作前报错。
        let error = backend
            .execute_profile(AppCommand::InstallHelper)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::PlatformFailed);
        assert!(error.message.contains("packaged Verge.app"));

        // 状态查询走配置的 helper socket（不存在 → NotInstalled），不触达真实系统。
        let status = backend
            .execute_profile(AppCommand::GetHelperStatus)
            .unwrap();
        assert!(matches!(
            status.output,
            AppCommandOutput::HelperStatus(crate::domain::HelperStatus::NotInstalled)
        ));
        let _ = fs::remove_dir_all(data_dir);
    }

    #[test]
    #[ignore = "requires MIHOMO_BIN pointing to the pinned sidecar"]
    fn shutdown_signal_terminates_the_real_mihomo_child() {
        let binary = PathBuf::from(std::env::var_os("MIHOMO_BIN").expect("MIHOMO_BIN is required"));
        let data_dir = std::env::temp_dir().join(format!(
            "verge-runtime-real-shutdown-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&data_dir).unwrap();
        let manifest =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/mihomo/manifest.json");
        let controller = available_controller().unwrap();
        let mixed_port = available_controller().unwrap().port();
        let config = BackendConfig {
            binary,
            manifest,
            controller,
            secret: generate_secret().unwrap(),
            services: vec!["Wi-Fi".into()],
            recovery_path: data_dir.join("recovery.json"),
            helper_socket: data_dir.join("helper.sock"),
            data_dir: data_dir.clone(),
        };
        let mut backend = Backend::new(config).unwrap();
        let id = ProfileId::parse("exit-smoke").unwrap();
        backend
            .execute_profile(AppCommand::ImportProfile {
                id: id.clone(),
                name: "Exit Smoke".into(),
                yaml: format!(
                    "mixed-port: {mixed_port}\nmode: rule\nlog-level: warning\nipv6: false\nproxies: []\nproxy-groups: []\nrules:\n  - MATCH,DIRECT\n"
                ),
                source: ProfileSource::Local,
                update_policy: UpdatePolicy::Manual,
            })
            .unwrap();
        backend
            .execute_profile(AppCommand::SelectProfile { id })
            .unwrap();
        let pid = match backend
            .engine
            .as_ref()
            .expect("Mihomo engine should be running")
            .supervisor
            .state()
        {
            crate::mihomo::SupervisorState::Running { pid } => *pid,
            state => panic!("expected running Mihomo, got {state:?}"),
        };

        backend.shutdown();

        #[cfg(unix)]
        assert!(
            !std::process::Command::new("/bin/kill")
                .args(["-0", &pid.to_string()])
                .status()
                .unwrap()
                .success(),
            "Mihomo process {pid} survived backend shutdown"
        );
        let _ = fs::remove_dir_all(data_dir);
    }
}
