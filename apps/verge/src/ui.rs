use std::collections::{HashMap, HashSet};

use crate::domain::{
    AppCommand, AppCommandOutput, AppCommandResult, AppError, AppUpdateStatus, ApplicationSettings,
    ApplicationSettingsSnapshot, CommandContext, CommandRisk, ConnectionSnapshot, HelperStatus,
    LogEvent, MemoryEvent, NetworkSettings, Profile, ProfileId, ProfileSource, ProviderKind,
    ProviderSummary, ProxyEndpoint, ProxyGroup, RealtimeEvent, RealtimeTopic, RuleEntry, RunMode,
    RuntimeCommand, RuntimeCommandOutput, RuntimeCommandResult, RuntimeSettings,
    SettingsImportPreview, SettingsScope, SystemProxyCommand, SystemProxyCommandResult,
    SystemProxyState, TrafficEvent, UpdatePolicy,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Page {
    #[default]
    Home,
    Proxies,
    Profiles,
    Connections,
    Rules,
    Logs,
    Settings,
}

#[cfg(test)]
mod envelope_tests {
    use super::*;
    use crate::domain::{CommandActor, CommandContext};

    fn envelope(request_id: u64) -> UiRequestEnvelope {
        UiRequestEnvelope {
            request_id,
            operation_id: request_id,
            operation_index: 0,
            operation_len: 1,
            context: CommandContext {
                actor: CommandActor::UserInterface,
                approval: None,
            },
            request: UiRequest::Runtime(RuntimeCommand::GetMode),
        }
    }

    fn mode_response(request_id: u64, mode: RunMode) -> UiResponseEnvelope {
        UiResponseEnvelope {
            request_id: Some(request_id),
            operation_id: Some(request_id),
            response: UiResponse::Runtime {
                request: RuntimeCommand::GetMode,
                result: Ok(RuntimeCommandResult {
                    output: RuntimeCommandOutput::Mode(mode),
                    summary: "loaded".into(),
                }),
            },
        }
    }

    #[test]
    fn duplicate_pending_requests_remain_pending_until_all_finish() {
        let mut state = UiState::default();
        state.begin_envelope(&envelope(1));
        state.begin_envelope(&envelope(2));

        state.apply_response_envelope(mode_response(1, RunMode::Rule));
        assert!(state.pending.contains("mode"));
        assert_eq!(
            state.mode, None,
            "stale read must not overwrite newer intent"
        );

        state.apply_response_envelope(mode_response(2, RunMode::Global));
        assert!(!state.pending.contains("mode"));
        assert_eq!(state.mode, Some(RunMode::Global));
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CoreStatus {
    #[default]
    Unknown,
    Running,
    Offline,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum UiRequest {
    Profile(AppCommand),
    Runtime(RuntimeCommand),
    SystemProxy(SystemProxyCommand),
}

impl UiRequest {
    pub fn risk(&self) -> CommandRisk {
        match self {
            Self::Profile(command) => command.risk(),
            Self::Runtime(command) => command.risk(),
            Self::SystemProxy(command) => command.risk(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UiRequestEnvelope {
    pub request_id: u64,
    pub operation_id: u64,
    pub operation_index: usize,
    pub operation_len: usize,
    pub context: CommandContext,
    pub request: UiRequest,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum UiResponse {
    Profile {
        request: AppCommand,
        result: Result<AppCommandResult, AppError>,
    },
    Runtime {
        request: RuntimeCommand,
        result: Result<RuntimeCommandResult, AppError>,
    },
    SystemProxy {
        request: SystemProxyCommand,
        result: Result<SystemProxyCommandResult, AppError>,
    },
    Realtime(RealtimeEvent),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UiResponseEnvelope {
    pub request_id: Option<u64>,
    pub operation_id: Option<u64>,
    pub response: UiResponse,
}

impl UiResponseEnvelope {
    pub fn for_request(request: &UiRequestEnvelope, response: UiResponse) -> Self {
        Self {
            request_id: Some(request.request_id),
            operation_id: Some(request.operation_id),
            response,
        }
    }

    pub fn realtime(response: UiResponse) -> Self {
        Self {
            request_id: None,
            operation_id: None,
            response,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UiAction {
    Navigate(Page),
    RefreshHome,
    RefreshProxies,
    RefreshRules,
    RefreshProfiles,
    RefreshSettings,
    UpdateSettings(ApplicationSettings),
    UpdateNetworkSettings(NetworkSettings),
    ExportApplicationSettings {
        destination: String,
    },
    PreviewSettingsImport {
        source: String,
    },
    ImportApplicationSettings {
        source: String,
    },
    ResetSettingsScope(SettingsScope),
    ExportDiagnostics {
        destination: String,
    },
    ExportEncryptedBackup {
        passphrase: String,
    },
    RestoreEncryptedBackup {
        passphrase: String,
    },
    UpdateMihomo,
    /// 检查应用自身更新（只读，返回当前/最新版本与是否有更新）。
    CheckAppUpdate,
    /// 下载并替换当前 .app（高权限写入，需确认弹窗 + 显式授权；重启后生效）。
    UpdateApplication,
    /// 重启应用使更新生效（高权限写入，需确认弹窗 + 显式授权）。
    RestartApplication,
    /// 安装/修复特权 helper（高权限写入，提权由系统授权弹窗完成）。
    InstallHelper,
    /// 卸载特权 helper（破坏性系统变更，需确认弹窗 + 显式授权）。
    UninstallHelper,
    LoadProfileYaml(ProfileId),
    ImportProfile {
        id: ProfileId,
        name: String,
        yaml: String,
        source: ProfileSource,
        update_policy: UpdatePolicy,
    },
    ImportRemoteProfile {
        id: ProfileId,
        name: String,
        url: String,
        update_policy: UpdatePolicy,
        user_agent: Option<String>,
    },
    SelectProfile(ProfileId),
    UpdateProfileYaml {
        id: ProfileId,
        yaml: String,
    },
    LoadMergeConfig,
    SaveMergeConfig {
        yaml: String,
    },
    LoadMergedYaml(ProfileId),
    DeleteProfile(ProfileId),
    UpdateRemoteProfile(ProfileId),
    SetProfileUpdatePolicy {
        id: ProfileId,
        update_policy: UpdatePolicy,
    },
    SetMode(RunMode),
    SetSystemProxy {
        enabled: bool,
        services: Vec<String>,
        endpoint: ProxyEndpoint,
    },
    SetSocksProxy {
        enabled: bool,
        endpoint: ProxyEndpoint,
    },
    SetAutoProxy {
        url: Option<String>,
    },
    SetProxyBypass {
        domains: Vec<String>,
    },
    SelectProxy {
        group: String,
        proxy: String,
    },
    TestDelay {
        proxy: String,
        url: String,
        timeout_ms: u32,
    },
    CloseConnection(String),
    UpdateProvider {
        kind: ProviderKind,
        name: String,
    },
}

impl UiAction {
    pub fn requests(self) -> Vec<UiRequest> {
        match self {
            Self::Navigate(_) => Vec::new(),
            Self::RefreshHome => vec![
                UiRequest::Profile(AppCommand::GetRuntimeSettings),
                UiRequest::Runtime(RuntimeCommand::GetMode),
                UiRequest::SystemProxy(SystemProxyCommand::GetState),
                UiRequest::Runtime(RuntimeCommand::StartRealtime {
                    topics: vec![
                        RealtimeTopic::Traffic,
                        RealtimeTopic::Memory,
                        RealtimeTopic::Connections,
                        RealtimeTopic::Logs,
                    ],
                }),
            ],
            Self::RefreshProxies => vec![UiRequest::Runtime(RuntimeCommand::ListProxyGroups)],
            Self::RefreshRules => vec![
                UiRequest::Runtime(RuntimeCommand::ListRules),
                UiRequest::Runtime(RuntimeCommand::ListProviders),
            ],
            Self::RefreshProfiles => vec![UiRequest::Profile(AppCommand::ListProfiles)],
            Self::RefreshSettings => vec![
                UiRequest::Profile(AppCommand::GetApplicationSettings),
                UiRequest::Profile(AppCommand::GetHelperStatus),
                UiRequest::Profile(AppCommand::GetRuntimeSettings),
                UiRequest::Runtime(RuntimeCommand::GetNetworkSettings),
                UiRequest::SystemProxy(SystemProxyCommand::GetState),
            ],
            Self::UpdateSettings(settings) => vec![
                UiRequest::Profile(AppCommand::UpdateApplicationSettings { settings }),
                UiRequest::Profile(AppCommand::GetApplicationSettings),
            ],
            Self::UpdateNetworkSettings(settings) => {
                vec![UiRequest::Runtime(RuntimeCommand::SetNetworkSettings {
                    settings,
                })]
            }
            Self::ExportDiagnostics { destination } => {
                vec![UiRequest::Profile(AppCommand::ExportDiagnostics {
                    destination,
                })]
            }
            Self::ExportApplicationSettings { destination } => {
                vec![UiRequest::Profile(AppCommand::ExportApplicationSettings {
                    destination,
                })]
            }
            // 导入先预览差异,由用户确认后再发 ImportApplicationSettings。
            Self::PreviewSettingsImport { source } => {
                vec![UiRequest::Profile(
                    AppCommand::PreviewApplicationSettingsImport { source },
                )]
            }
            Self::ImportApplicationSettings { source } => {
                vec![UiRequest::Profile(AppCommand::ImportApplicationSettings {
                    source,
                })]
            }
            Self::ResetSettingsScope(scope) => {
                vec![UiRequest::Profile(
                    AppCommand::ResetApplicationSettingsScope { scope },
                )]
            }
            Self::ExportEncryptedBackup { passphrase } => {
                vec![UiRequest::Profile(AppCommand::ExportEncryptedBackup {
                    passphrase,
                })]
            }
            Self::RestoreEncryptedBackup { passphrase } => vec![
                UiRequest::Profile(AppCommand::RestoreEncryptedBackup { passphrase }),
                UiRequest::Profile(AppCommand::ListProfiles),
                UiRequest::Profile(AppCommand::GetApplicationSettings),
            ],
            Self::UpdateMihomo => vec![UiRequest::Profile(AppCommand::UpdateMihomo)],
            Self::CheckAppUpdate => vec![UiRequest::Profile(AppCommand::CheckAppUpdate)],
            // 更新与重启是单命令事务；失败时错误 toast 已足够，不做跟随刷新。
            Self::UpdateApplication => vec![UiRequest::Profile(AppCommand::UpdateApplication)],
            Self::RestartApplication => {
                vec![UiRequest::Profile(AppCommand::RestartApplication)]
            }
            // 安装/卸载完成后立刻刷新 Helper 状态行。
            Self::InstallHelper => vec![
                UiRequest::Profile(AppCommand::InstallHelper),
                UiRequest::Profile(AppCommand::GetHelperStatus),
            ],
            Self::UninstallHelper => vec![
                UiRequest::Profile(AppCommand::UninstallHelper),
                UiRequest::Profile(AppCommand::GetHelperStatus),
            ],
            Self::LoadProfileYaml(id) => {
                vec![UiRequest::Profile(AppCommand::GetProfileYaml { id })]
            }
            Self::ImportProfile {
                id,
                name,
                yaml,
                source,
                update_policy,
            } => vec![
                UiRequest::Profile(AppCommand::ImportProfile {
                    id,
                    name,
                    yaml,
                    source,
                    update_policy,
                }),
                UiRequest::Profile(AppCommand::ListProfiles),
            ],
            Self::ImportRemoteProfile {
                id,
                name,
                url,
                update_policy,
                user_agent,
            } => vec![
                UiRequest::Profile(AppCommand::ImportRemoteProfile {
                    id,
                    name,
                    url,
                    update_policy,
                    user_agent,
                }),
                UiRequest::Profile(AppCommand::ListProfiles),
            ],
            Self::SelectProfile(id) => vec![
                UiRequest::Profile(AppCommand::SelectProfile { id }),
                UiRequest::Profile(AppCommand::ListProfiles),
                UiRequest::Profile(AppCommand::GetRuntimeSettings),
                // 启用后主动确认 Mihomo 控制链路，并立即刷新运行模式和内核状态。
                UiRequest::Runtime(RuntimeCommand::GetMode),
            ],
            Self::UpdateProfileYaml { id, yaml } => vec![
                UiRequest::Profile(AppCommand::UpdateProfileYaml { id, yaml }),
                UiRequest::Profile(AppCommand::ListProfiles),
            ],
            Self::LoadMergeConfig => vec![UiRequest::Profile(AppCommand::GetMergeConfig)],
            Self::SaveMergeConfig { yaml } => vec![
                UiRequest::Profile(AppCommand::UpdateMergeConfig { yaml }),
                UiRequest::Profile(AppCommand::GetMergeConfig),
            ],
            Self::LoadMergedYaml(id) => {
                vec![UiRequest::Profile(AppCommand::GetMergedProfileYaml { id })]
            }
            Self::DeleteProfile(id) => vec![
                UiRequest::Profile(AppCommand::DeleteProfile { id }),
                UiRequest::Profile(AppCommand::ListProfiles),
            ],
            Self::UpdateRemoteProfile(id) => vec![
                UiRequest::Profile(AppCommand::UpdateRemoteProfile { id }),
                UiRequest::Profile(AppCommand::ListProfiles),
            ],
            Self::SetProfileUpdatePolicy { id, update_policy } => vec![
                UiRequest::Profile(AppCommand::SetProfileUpdatePolicy { id, update_policy }),
                UiRequest::Profile(AppCommand::ListProfiles),
            ],
            Self::SetMode(mode) => {
                vec![UiRequest::Runtime(RuntimeCommand::SetMode { mode })]
            }
            Self::SetSystemProxy {
                enabled,
                services,
                endpoint,
            } => vec![UiRequest::SystemProxy(if enabled {
                SystemProxyCommand::Enable { services, endpoint }
            } else {
                SystemProxyCommand::Disable
            })],
            Self::SetSocksProxy { enabled, endpoint } => {
                vec![UiRequest::SystemProxy(SystemProxyCommand::SetSocks {
                    enabled,
                    endpoint,
                })]
            }
            Self::SetAutoProxy { url } => {
                vec![UiRequest::SystemProxy(SystemProxyCommand::SetAutoProxy {
                    url,
                })]
            }
            Self::SetProxyBypass { domains } => {
                vec![UiRequest::SystemProxy(SystemProxyCommand::SetProxyBypass {
                    domains,
                })]
            }
            Self::SelectProxy { group, proxy } => {
                vec![UiRequest::Runtime(RuntimeCommand::SelectProxy {
                    group,
                    proxy,
                })]
            }
            Self::TestDelay {
                proxy,
                url,
                timeout_ms,
            } => vec![UiRequest::Runtime(RuntimeCommand::TestProxyDelay {
                proxy,
                url,
                timeout_ms,
            })],
            Self::CloseConnection(id) => {
                vec![UiRequest::Runtime(RuntimeCommand::CloseConnection { id })]
            }
            Self::UpdateProvider { kind, name } => vec![
                UiRequest::Runtime(RuntimeCommand::UpdateProvider { kind, name }),
                UiRequest::Runtime(RuntimeCommand::ListProviders),
            ],
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct UiState {
    pub page: Page,
    pub core_status: CoreStatus,
    pub mode: Option<RunMode>,
    pub traffic: Option<TrafficEvent>,
    pub memory: Option<MemoryEvent>,
    pub system_proxy: Option<SystemProxyState>,
    pub proxy_groups: Vec<ProxyGroup>,
    pub rules: Vec<RuleEntry>,
    pub providers: Vec<ProviderSummary>,
    pub profiles: Vec<Profile>,
    pub selected_profile: Option<ProfileId>,
    pub profile_yaml: Option<(ProfileId, String)>,
    /// 全局 Merge 配置的 YAML 文本(供编辑)。
    pub merge_yaml: Option<String>,
    /// merge 增强后的合并结果预览(不含私有 controller 注入)。
    pub merged_yaml: Option<(ProfileId, String)>,
    pub runtime_settings: Option<RuntimeSettings>,
    pub application_settings: Option<ApplicationSettingsSnapshot>,
    pub network_settings: Option<NetworkSettings>,
    pub helper_status: Option<HelperStatus>,
    pub mihomo_version: Option<String>,
    /// 最近一次应用更新检查结果（当前/最新版本与是否有更新）。
    pub app_update: Option<AppUpdateStatus>,
    /// 已下载并就位、等待重启生效的应用版本。
    pub app_update_installed: Option<String>,
    pub last_diagnostic_path: Option<String>,
    /// 最近一次设置导入预览的结果,确认弹窗据此展示差异。
    pub settings_import_preview: Option<SettingsImportPreview>,
    pub connections: Option<ConnectionSnapshot>,
    pub logs: Vec<LogEvent>,
    pub delays: HashMap<String, u32>,
    /// 正在测速的节点名，用于按节点渲染加载态。
    pub delay_pending: HashSet<String>,
    pub pending: HashSet<&'static str>,
    pending_requests: HashMap<u64, &'static str>,
    latest_requests: HashMap<&'static str, u64>,
    pub last_error: Option<AppError>,
    last_error_from_write: bool,
}

impl UiState {
    pub fn begin_envelope(&mut self, envelope: &UiRequestEnvelope) {
        let key = request_key(&envelope.request);
        self.pending_requests.insert(envelope.request_id, key);
        self.latest_requests.insert(key, envelope.request_id);
        self.begin(&envelope.request);
    }

    pub fn navigate(&mut self, page: Page) {
        self.page = page;
    }

    pub fn begin(&mut self, request: &UiRequest) {
        self.pending.insert(request_key(request));
        if let UiRequest::Runtime(RuntimeCommand::TestProxyDelay { proxy, .. }) = request {
            self.delay_pending.insert(proxy.clone());
        }
        self.last_error = None;
        self.last_error_from_write = false;
    }

    pub fn set_error(&mut self, error: AppError) {
        self.last_error = Some(error);
        self.last_error_from_write = true;
    }

    pub fn clear_error(&mut self) {
        self.last_error = None;
        self.last_error_from_write = false;
    }

    pub fn fail(&mut self, request: &UiRequest, error: AppError) {
        self.pending.remove(request_key(request));
        if let UiRequest::Runtime(RuntimeCommand::TestProxyDelay { proxy, .. }) = request {
            self.delay_pending.remove(proxy);
        }
        if matches!(request, UiRequest::Runtime(_)) {
            self.core_status = CoreStatus::Offline;
        }
        if is_write_request(request) {
            self.set_error(error);
        } else if !self.last_error_from_write {
            self.last_error = Some(error);
        }
    }

    pub fn apply_runtime(&mut self, request: &RuntimeCommand, result: RuntimeCommandResult) {
        self.pending
            .remove(request_key(&UiRequest::Runtime(request.clone())));
        self.core_status = CoreStatus::Running;
        match result.output {
            RuntimeCommandOutput::None => {
                if let RuntimeCommand::SelectProxy { group, proxy } = request
                    && let Some(proxy_group) = self
                        .proxy_groups
                        .iter_mut()
                        .find(|candidate| candidate.name == *group)
                {
                    proxy_group.selected = Some(proxy.clone());
                }
            }
            RuntimeCommandOutput::Mode(mode) => self.mode = Some(mode),
            RuntimeCommandOutput::ProxyGroups(groups) => self.proxy_groups = groups,
            RuntimeCommandOutput::Rules(rules) => self.rules = rules,
            RuntimeCommandOutput::Providers(providers) => self.providers = providers,
            RuntimeCommandOutput::NetworkSettings(settings) => {
                self.network_settings = Some(settings);
            }
            RuntimeCommandOutput::Delay(delay) => {
                if let RuntimeCommand::TestProxyDelay { proxy, .. } = request {
                    self.delay_pending.remove(proxy);
                    self.delays.insert(proxy.clone(), delay);
                }
            }
            RuntimeCommandOutput::Realtime(events) => {
                for event in events {
                    self.apply_realtime(event);
                }
            }
        }
    }

    pub fn apply_profile(&mut self, request: &AppCommand, result: AppCommandResult) {
        self.pending
            .remove(request_key(&UiRequest::Profile(request.clone())));
        match result.output {
            AppCommandOutput::RuntimeSettings(settings) => {
                self.runtime_settings = Some(settings);
            }
            AppCommandOutput::ApplicationSettings(settings) => {
                self.application_settings = Some(settings);
            }
            AppCommandOutput::HelperStatus(status) => {
                self.helper_status = Some(status);
            }
            AppCommandOutput::DiagnosticsExported { path } => {
                self.last_diagnostic_path = Some(path);
            }
            AppCommandOutput::ApplicationSettingsExported { path } => {
                self.last_diagnostic_path = Some(path);
            }
            AppCommandOutput::ApplicationSettingsImportPreview(preview) => {
                self.settings_import_preview = Some(preview);
            }
            AppCommandOutput::EncryptedBackupExported { path } => {
                self.last_diagnostic_path = Some(path);
            }
            AppCommandOutput::MihomoUpdated { version } => {
                self.mihomo_version = Some(version);
            }
            AppCommandOutput::AppUpdateStatus(status) => {
                self.app_update = Some(status);
            }
            AppCommandOutput::ApplicationUpdated { version, .. } => {
                // 更新已就位：清掉检查快照，UI 改显示“重启生效”状态。
                self.app_update_installed = Some(version);
                self.app_update = None;
            }
            AppCommandOutput::Profiles { profiles, selected } => {
                self.profiles = profiles;
                self.selected_profile = selected;
            }
            AppCommandOutput::ProfileYaml { id, yaml } => {
                self.profile_yaml = Some((id, yaml));
            }
            AppCommandOutput::MergeConfigYaml { yaml } => {
                self.merge_yaml = Some(yaml);
            }
            AppCommandOutput::MergedProfileYaml { id, yaml } => {
                self.merged_yaml = Some((id, yaml));
            }
            AppCommandOutput::None => {
                // SelectProfile 本身已经是事务提交点；不要依赖后续 ListProfiles
                // 才更新选中态，否则跟随读取失败时会出现“成功但界面未启用”。
                if let AppCommand::SelectProfile { id } = request {
                    self.selected_profile = Some(id.clone());
                }
            }
        }
    }

    pub fn apply_system_proxy(
        &mut self,
        request: &SystemProxyCommand,
        result: SystemProxyCommandResult,
    ) {
        self.pending
            .remove(request_key(&UiRequest::SystemProxy(request.clone())));
        self.system_proxy = Some(result.state);
    }

    pub fn apply_realtime(&mut self, event: RealtimeEvent) {
        match event {
            RealtimeEvent::Traffic(traffic) => self.traffic = Some(traffic),
            RealtimeEvent::Memory(memory) => self.memory = Some(memory),
            RealtimeEvent::Connections(connections) => self.connections = Some(connections),
            RealtimeEvent::Log(log) => {
                let limit = self
                    .application_settings
                    .as_ref()
                    .map_or(500, |snapshot| usize::from(snapshot.settings.log_limit));
                if self.logs.len() >= limit {
                    self.logs.remove(0);
                }
                self.logs.push(log);
            }
            RealtimeEvent::Reconnecting { .. } => self.core_status = CoreStatus::Offline,
        }
    }

    pub fn apply_response(&mut self, response: UiResponse) {
        match response {
            UiResponse::Profile { request, result } => match result {
                Ok(result) => self.apply_profile(&request, result),
                Err(error) => self.fail(&UiRequest::Profile(request), error),
            },
            UiResponse::Runtime { request, result } => match result {
                Ok(result) => self.apply_runtime(&request, result),
                Err(error) => self.fail(&UiRequest::Runtime(request), error),
            },
            UiResponse::SystemProxy { request, result } => match result {
                Ok(result) => self.apply_system_proxy(&request, result),
                Err(error) => self.fail(&UiRequest::SystemProxy(request), error),
            },
            UiResponse::Realtime(event) => self.apply_realtime(event),
        }
    }

    pub fn apply_response_envelope(&mut self, envelope: UiResponseEnvelope) {
        let Some(request_id) = envelope.request_id else {
            self.apply_response(envelope.response);
            return;
        };
        let Some(request) = response_request(&envelope.response) else {
            self.apply_response(envelope.response);
            return;
        };
        let key = request_key(&request);
        let stale = self
            .latest_requests
            .get(key)
            .is_some_and(|latest| *latest != request_id);
        self.pending_requests.remove(&request_id);
        if !stale || is_write_request(&request) {
            self.apply_response(envelope.response);
        }
        if self
            .pending_requests
            .values()
            .any(|pending| *pending == key)
        {
            self.pending.insert(key);
        } else {
            self.pending.remove(key);
            self.latest_requests.remove(key);
        }
    }

    pub fn system_proxy_enabled(&self) -> bool {
        self.system_proxy.as_ref().is_some_and(|state| {
            !state.services.is_empty()
                && state.services.iter().all(|service| {
                    service.web.enabled
                        && service.secure_web.enabled
                        && service.web.endpoint == service.secure_web.endpoint
                })
        })
    }
}

fn response_request(response: &UiResponse) -> Option<UiRequest> {
    match response {
        UiResponse::Profile { request, .. } => Some(UiRequest::Profile(request.clone())),
        UiResponse::Runtime { request, .. } => Some(UiRequest::Runtime(request.clone())),
        UiResponse::SystemProxy { request, .. } => Some(UiRequest::SystemProxy(request.clone())),
        UiResponse::Realtime(_) => None,
    }
}

fn is_write_request(request: &UiRequest) -> bool {
    match request {
        UiRequest::SystemProxy(
            SystemProxyCommand::Enable { .. }
            | SystemProxyCommand::Disable
            | SystemProxyCommand::SetSocks { .. }
            | SystemProxyCommand::SetAutoProxy { .. }
            | SystemProxyCommand::SetProxyBypass { .. },
        ) => true,
        UiRequest::Runtime(RuntimeCommand::SelectProxy { .. }) => true,
        _ => request_key(request).ends_with("_write"),
    }
}

fn request_key(request: &UiRequest) -> &'static str {
    match request {
        UiRequest::Profile(AppCommand::GetRuntimeSettings) => "runtime_settings",
        UiRequest::Profile(AppCommand::GetApplicationSettings) => "application_settings",
        UiRequest::Profile(AppCommand::GetHelperStatus) => "helper_status",
        UiRequest::Profile(AppCommand::CheckAppUpdate) => "app_update",
        UiRequest::Profile(AppCommand::UpdateApplication | AppCommand::RestartApplication) => {
            "app_update_write"
        }
        UiRequest::Profile(AppCommand::QuitApplication) => "application_lifecycle_write",
        UiRequest::Profile(AppCommand::InstallHelper | AppCommand::UninstallHelper) => {
            "helper_write"
        }
        UiRequest::Profile(
            AppCommand::UpdateApplicationSettings { .. }
            | AppCommand::ExportApplicationSettings { .. }
            | AppCommand::ImportApplicationSettings { .. }
            | AppCommand::ResetApplicationSettingsScope { .. }
            | AppCommand::ExportDiagnostics { .. }
            | AppCommand::ExportEncryptedBackup { .. }
            | AppCommand::RestoreEncryptedBackup { .. }
            | AppCommand::UpdateMihomo,
        ) => "application_settings_write",
        UiRequest::Profile(AppCommand::PreviewApplicationSettingsImport { .. }) => {
            "settings_import_preview"
        }
        UiRequest::Profile(AppCommand::ListProfiles) => "profiles",
        UiRequest::Profile(AppCommand::GetProfileYaml { .. }) => "profile_yaml",
        UiRequest::Profile(AppCommand::GetMergeConfig) => "merge_config",
        UiRequest::Profile(AppCommand::GetMergedProfileYaml { .. }) => "merged_yaml",
        UiRequest::Profile(
            AppCommand::ImportProfile { .. }
            | AppCommand::ImportRemoteProfile { .. }
            | AppCommand::SelectProfile { .. }
            | AppCommand::UpdateProfileYaml { .. }
            | AppCommand::UpdateMergeConfig { .. }
            | AppCommand::UpdateRemoteProfile { .. }
            | AppCommand::SetProfileUpdatePolicy { .. }
            | AppCommand::DeleteProfile { .. },
        ) => "profile_write",
        UiRequest::Runtime(RuntimeCommand::GetMode | RuntimeCommand::SetMode { .. }) => "mode",
        UiRequest::Runtime(RuntimeCommand::ListProxyGroups) => "proxy_groups",
        UiRequest::Runtime(RuntimeCommand::ListRules) => "rules",
        UiRequest::Runtime(RuntimeCommand::ListProviders) => "providers",
        UiRequest::Runtime(RuntimeCommand::UpdateProvider { .. }) => "provider_write",
        UiRequest::Runtime(RuntimeCommand::GetNetworkSettings) => "network_settings",
        UiRequest::Runtime(RuntimeCommand::SetNetworkSettings { .. }) => "network_settings_write",
        UiRequest::Runtime(RuntimeCommand::SelectProxy { .. }) => "select_proxy",
        UiRequest::Runtime(RuntimeCommand::TestProxyDelay { .. }) => "delay",
        UiRequest::Runtime(RuntimeCommand::CloseConnection { .. }) => "connections_write",
        UiRequest::Runtime(
            RuntimeCommand::StartRealtime { .. }
            | RuntimeCommand::StopRealtime
            | RuntimeCommand::DrainRealtime,
        ) => "realtime",
        UiRequest::SystemProxy(_) => "system_proxy",
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::{
        ErrorCode, ProxyProtocolState, RuntimeCommandOutput, SystemProxyServiceState,
    };

    use super::*;

    #[test]
    fn home_refresh_only_dispatches_typed_application_commands() {
        assert_eq!(
            UiAction::RefreshHome.requests(),
            [
                UiRequest::Profile(AppCommand::GetRuntimeSettings),
                UiRequest::Runtime(RuntimeCommand::GetMode),
                UiRequest::SystemProxy(SystemProxyCommand::GetState),
                UiRequest::Runtime(RuntimeCommand::StartRealtime {
                    topics: vec![
                        RealtimeTopic::Traffic,
                        RealtimeTopic::Memory,
                        RealtimeTopic::Connections,
                        RealtimeTopic::Logs,
                    ]
                })
            ]
        );
    }

    #[test]
    fn runtime_results_update_stable_ui_state() {
        let mut state = UiState::default();
        let request = RuntimeCommand::GetMode;
        state.begin(&UiRequest::Runtime(request.clone()));
        state.apply_runtime(
            &request,
            RuntimeCommandResult {
                output: RuntimeCommandOutput::Mode(RunMode::Global),
                summary: "loaded".into(),
            },
        );
        state.apply_realtime(RealtimeEvent::Traffic(TrafficEvent { up: 10, down: 20 }));
        assert_eq!(state.mode, Some(RunMode::Global));
        assert_eq!(state.traffic, Some(TrafficEvent { up: 10, down: 20 }));
        assert_eq!(state.core_status, CoreStatus::Running);
        assert!(!state.pending.contains("mode"));
    }

    #[test]
    fn profile_results_update_list_selection_and_editor() {
        let id = ProfileId::parse("daily").unwrap();
        let profile = Profile::new(
            id.clone(),
            "Daily",
            ProfileSource::Local,
            UpdatePolicy::Manual,
            100,
            None,
        )
        .unwrap();
        let mut state = UiState::default();
        state.apply_profile(
            &AppCommand::ListProfiles,
            AppCommandResult {
                output: AppCommandOutput::Profiles {
                    profiles: vec![profile],
                    selected: Some(id.clone()),
                },
                summary: "loaded".into(),
            },
        );
        state.apply_profile(
            &AppCommand::GetProfileYaml { id: id.clone() },
            AppCommandResult {
                output: AppCommandOutput::ProfileYaml {
                    id: id.clone(),
                    yaml: "mode: rule\n".into(),
                },
                summary: "loaded".into(),
            },
        );
        assert_eq!(state.selected_profile, Some(id.clone()));
        assert_eq!(state.profiles.len(), 1);
        assert_eq!(state.profile_yaml, Some((id, "mode: rule\n".into())));
    }

    #[test]
    fn realtime_connections_and_logs_are_bounded_in_ui_state() {
        let mut state = UiState::default();
        state.apply_realtime(RealtimeEvent::Connections(ConnectionSnapshot {
            upload_total: 10,
            download_total: 20,
            connection_count: 2,
            connections: Vec::new(),
        }));
        for index in 0..501 {
            state.apply_realtime(RealtimeEvent::Log(LogEvent {
                level: "info".into(),
                payload: index.to_string(),
            }));
        }
        assert_eq!(state.connections.as_ref().unwrap().connection_count, 2);
        assert_eq!(state.logs.len(), 500);
        assert_eq!(state.logs.first().unwrap().payload, "1");
        assert_eq!(state.logs.last().unwrap().payload, "500");
    }

    #[test]
    fn selected_proxy_and_delay_are_merged_without_reloading_the_page() {
        let mut state = UiState {
            proxy_groups: vec![ProxyGroup {
                name: "Select".into(),
                kind: "Selector".into(),
                selected: Some("A".into()),
                members: vec!["A".into(), "B".into()],
            }],
            ..UiState::default()
        };
        let selection = RuntimeCommand::SelectProxy {
            group: "Select".into(),
            proxy: "B".into(),
        };
        state.apply_runtime(
            &selection,
            RuntimeCommandResult {
                output: RuntimeCommandOutput::None,
                summary: "selected".into(),
            },
        );
        let delay = RuntimeCommand::TestProxyDelay {
            proxy: "B".into(),
            url: "https://example.com".into(),
            timeout_ms: 5_000,
        };
        state.apply_runtime(
            &delay,
            RuntimeCommandResult {
                output: RuntimeCommandOutput::Delay(42),
                summary: "tested".into(),
            },
        );
        assert_eq!(state.proxy_groups[0].selected.as_deref(), Some("B"));
        assert_eq!(state.delays.get("B"), Some(&42));
    }

    #[test]
    fn successful_profile_selection_updates_ui_before_follow_up_reads() {
        let id = ProfileId::parse("daily").unwrap();
        let mut state = UiState::default();
        state.apply_profile(
            &AppCommand::SelectProfile { id: id.clone() },
            AppCommandResult {
                output: AppCommandOutput::None,
                summary: "selected".into(),
            },
        );
        assert_eq!(state.selected_profile, Some(id));
    }

    #[test]
    fn profile_selection_requests_runtime_confirmation() {
        let id = ProfileId::parse("daily").unwrap();
        let requests = UiAction::SelectProfile(id).requests();
        assert!(matches!(
            requests.last(),
            Some(UiRequest::Runtime(RuntimeCommand::GetMode))
        ));
    }

    #[test]
    fn settings_transfer_actions_map_to_profile_commands() {
        let export = UiAction::ExportApplicationSettings {
            destination: "/tmp/settings.json".into(),
        }
        .requests();
        assert_eq!(
            export,
            [UiRequest::Profile(AppCommand::ExportApplicationSettings {
                destination: "/tmp/settings.json".into(),
            })]
        );

        let preview = UiAction::PreviewSettingsImport {
            source: "/tmp/settings.json".into(),
        }
        .requests();
        assert!(matches!(
            preview.as_slice(),
            [UiRequest::Profile(
                AppCommand::PreviewApplicationSettingsImport { .. }
            )]
        ));
        assert!(!is_write_request(&preview[0]));

        for action in [
            UiAction::ImportApplicationSettings {
                source: "/tmp/settings.json".into(),
            },
            UiAction::ResetSettingsScope(SettingsScope::Network),
        ] {
            let requests = action.requests();
            assert_eq!(requests.len(), 1);
            assert!(is_write_request(&requests[0]));
        }
    }

    #[test]
    fn settings_import_preview_is_stored_for_confirmation_dialog() {
        let mut state = UiState::default();
        let preview = SettingsImportPreview {
            settings: ApplicationSettings::default(),
            changes: vec![crate::domain::SettingsFieldChange {
                field: "theme".into(),
                old: "system".into(),
                new: "dark".into(),
            }],
        };
        state.apply_profile(
            &AppCommand::PreviewApplicationSettingsImport {
                source: "/tmp/settings.json".into(),
            },
            AppCommandResult {
                output: AppCommandOutput::ApplicationSettingsImportPreview(preview.clone()),
                summary: "preview".into(),
            },
        );
        assert_eq!(state.settings_import_preview, Some(preview));
    }

    #[test]
    fn settings_refresh_loads_system_proxy_state() {
        let requests = UiAction::RefreshSettings.requests();
        assert!(requests.contains(&UiRequest::SystemProxy(SystemProxyCommand::GetState)));
        assert!(requests.contains(&UiRequest::Profile(AppCommand::GetRuntimeSettings)));
    }

    #[test]
    fn app_update_actions_map_to_typed_commands_and_update_state() {
        assert_eq!(
            UiAction::CheckAppUpdate.requests(),
            [UiRequest::Profile(AppCommand::CheckAppUpdate)]
        );
        assert!(!is_write_request(&UiAction::CheckAppUpdate.requests()[0]));
        for action in [UiAction::UpdateApplication, UiAction::RestartApplication] {
            let requests = action.requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(request_key(&requests[0]), "app_update_write");
            assert!(is_write_request(&requests[0]));
        }

        let mut state = UiState::default();
        state.apply_profile(
            &AppCommand::CheckAppUpdate,
            AppCommandResult {
                output: AppCommandOutput::AppUpdateStatus(AppUpdateStatus {
                    current_version: Some("0.1.0".into()),
                    latest_version: "0.2.0".into(),
                    update_available: true,
                    asset_url: "https://github.com/zzzgydi/verge/releases/download/v0.2.0/Verge-macos-arm64.zip".into(),
                }),
                summary: "checked".into(),
            },
        );
        assert!(state.app_update.as_ref().unwrap().update_available);
        state.apply_profile(
            &AppCommand::UpdateApplication,
            AppCommandResult {
                output: AppCommandOutput::ApplicationUpdated {
                    version: "0.2.0".into(),
                    restart_required: true,
                },
                summary: "updated".into(),
            },
        );
        assert_eq!(state.app_update_installed.as_deref(), Some("0.2.0"));
        assert!(state.app_update.is_none());
    }

    #[test]
    fn helper_install_and_uninstall_actions_refresh_status_afterwards() {
        for (action, command) in [
            (UiAction::InstallHelper, AppCommand::InstallHelper),
            (UiAction::UninstallHelper, AppCommand::UninstallHelper),
        ] {
            let requests = action.requests();
            assert_eq!(
                requests,
                [
                    UiRequest::Profile(command),
                    UiRequest::Profile(AppCommand::GetHelperStatus)
                ]
            );
            assert_eq!(request_key(&requests[0]), "helper_write");
        }
    }

    #[test]
    fn socks_pac_and_bypass_actions_map_to_privileged_system_proxy_writes() {
        let endpoint = ProxyEndpoint::new("127.0.0.1", 7891).unwrap();
        let socks = UiAction::SetSocksProxy {
            enabled: true,
            endpoint: endpoint.clone(),
        }
        .requests();
        assert_eq!(
            socks,
            [UiRequest::SystemProxy(SystemProxyCommand::SetSocks {
                enabled: true,
                endpoint,
            })]
        );
        assert!(is_write_request(&socks[0]));

        let pac = UiAction::SetAutoProxy {
            url: Some("http://127.0.0.1/proxy.pac".into()),
        }
        .requests();
        assert!(matches!(
            pac.as_slice(),
            [UiRequest::SystemProxy(SystemProxyCommand::SetAutoProxy {
                url: Some(_)
            })]
        ));
        assert!(is_write_request(&pac[0]));

        let bypass = UiAction::SetProxyBypass {
            domains: vec!["*.local".into()],
        }
        .requests();
        assert!(matches!(
            bypass.as_slice(),
            [UiRequest::SystemProxy(
                SystemProxyCommand::SetProxyBypass { .. }
            )]
        ));
        assert!(is_write_request(&bypass[0]));
    }

    #[test]
    fn system_proxy_requires_all_managed_protocols_to_match() {
        let endpoint = ProxyEndpoint::new("127.0.0.1", 7890).unwrap();
        let mut state = UiState::default();
        state.apply_system_proxy(
            &SystemProxyCommand::GetState,
            SystemProxyCommandResult {
                state: SystemProxyState {
                    services: vec![SystemProxyServiceState {
                        service: "Wi-Fi".into(),
                        web: ProxyProtocolState {
                            enabled: true,
                            endpoint: endpoint.clone(),
                        },
                        secure_web: ProxyProtocolState {
                            enabled: true,
                            endpoint,
                        },
                        socks: ProxyProtocolState {
                            enabled: false,
                            endpoint: ProxyEndpoint::new("127.0.0.1", 7891).unwrap(),
                        },
                        auto_proxy: crate::domain::AutoProxyState {
                            enabled: false,
                            url: None,
                        },
                        bypass: Vec::new(),
                    }],
                    recovery_pending: true,
                },
                summary: "loaded".into(),
            },
        );
        assert!(state.system_proxy_enabled());
    }

    #[test]
    fn failed_runtime_request_marks_core_offline_and_keeps_error_code() {
        let mut state = UiState::default();
        let request = UiRequest::Runtime(RuntimeCommand::ListProxyGroups);
        state.begin(&request);
        state.fail(
            &request,
            AppError::new(ErrorCode::CoreUnavailable, "offline"),
        );
        assert_eq!(state.core_status, CoreStatus::Offline);
        assert_eq!(state.last_error.unwrap().code, ErrorCode::CoreUnavailable);
    }

    #[test]
    fn delay_pending_is_tracked_per_proxy() {
        let mut state = UiState::default();
        let test_a = UiRequest::Runtime(RuntimeCommand::TestProxyDelay {
            proxy: "A".into(),
            url: "https://example.com".into(),
            timeout_ms: 5_000,
        });
        let test_b = UiRequest::Runtime(RuntimeCommand::TestProxyDelay {
            proxy: "B".into(),
            url: "https://example.com".into(),
            timeout_ms: 5_000,
        });
        state.begin(&test_a);
        state.begin(&test_b);
        assert!(state.delay_pending.contains("A"));
        assert!(state.delay_pending.contains("B"));

        state.apply_response(UiResponse::Runtime {
            request: RuntimeCommand::TestProxyDelay {
                proxy: "A".into(),
                url: "https://example.com".into(),
                timeout_ms: 5_000,
            },
            result: Ok(RuntimeCommandResult {
                output: RuntimeCommandOutput::Delay(42),
                summary: "tested".into(),
            }),
        });
        assert!(!state.delay_pending.contains("A"));
        assert_eq!(state.delays.get("A"), Some(&42));
        assert!(state.delay_pending.contains("B"));

        state.fail(
            &test_b,
            AppError::new(ErrorCode::CoreUnavailable, "offline"),
        );
        assert!(!state.delay_pending.contains("B"));
        assert_eq!(state.delays.get("B"), None);
    }

    #[test]
    fn read_failure_does_not_mask_earlier_write_failure() {
        let id = ProfileId::parse("daily").unwrap();
        let write = UiRequest::Profile(AppCommand::SelectProfile { id });
        let follow_up = UiRequest::Profile(AppCommand::GetRuntimeSettings);
        let mut state = UiState::default();
        state.begin(&write);
        state.begin(&follow_up);
        state.fail(
            &write,
            AppError::new(ErrorCode::ValidationFailed, "sidecar missing"),
        );
        state.fail(
            &follow_up,
            AppError::new(ErrorCode::NotFound, "no active profile"),
        );
        assert_eq!(
            state.last_error.as_ref().unwrap().message,
            "sidecar missing",
            "secondary read failure must not overwrite the write failure"
        );

        state.clear_error();
        state.fail(
            &follow_up,
            AppError::new(ErrorCode::NotFound, "no active profile"),
        );
        assert_eq!(
            state.last_error.unwrap().message,
            "no active profile",
            "read failure is shown when no write failure is pending"
        );
    }

    #[test]
    fn background_responses_share_one_reducer_path() {
        let mut state = UiState::default();
        let request = RuntimeCommand::GetMode;
        state.begin(&UiRequest::Runtime(request.clone()));
        state.apply_response(UiResponse::Runtime {
            request,
            result: Ok(RuntimeCommandResult {
                output: RuntimeCommandOutput::Mode(RunMode::Direct),
                summary: "loaded".into(),
            }),
        });
        assert_eq!(state.mode, Some(RunMode::Direct));
        assert_eq!(state.core_status, CoreStatus::Running);
    }

    #[test]
    fn merge_actions_map_to_typed_commands_and_update_state() {
        let save = UiAction::SaveMergeConfig {
            yaml: "rules: []\n".into(),
        }
        .requests();
        assert!(matches!(
            save.as_slice(),
            [
                UiRequest::Profile(AppCommand::UpdateMergeConfig { .. }),
                UiRequest::Profile(AppCommand::GetMergeConfig)
            ]
        ));
        assert!(is_write_request(&save[0]));

        let id = ProfileId::parse("daily").unwrap();
        let preview = UiAction::LoadMergedYaml(id.clone()).requests();
        assert!(matches!(
            preview.as_slice(),
            [UiRequest::Profile(AppCommand::GetMergedProfileYaml {
                id: _
            })]
        ));

        let mut state = UiState::default();
        state.apply_profile(
            &AppCommand::GetMergeConfig,
            AppCommandResult {
                output: AppCommandOutput::MergeConfigYaml {
                    yaml: "rules: []\n".into(),
                },
                summary: "loaded".into(),
            },
        );
        state.apply_profile(
            &AppCommand::GetMergedProfileYaml { id: id.clone() },
            AppCommandResult {
                output: AppCommandOutput::MergedProfileYaml {
                    id: id.clone(),
                    yaml: "mode: global\n".into(),
                },
                summary: "loaded".into(),
            },
        );
        assert_eq!(state.merge_yaml.as_deref(), Some("rules: []\n"));
        assert_eq!(state.merged_yaml, Some((id, "mode: global\n".into())));
    }
}
