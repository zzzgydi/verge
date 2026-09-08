mod network;
pub use network::{CoreNetworkSettings, ExternalControllerSettings};

use std::{error::Error, fmt, time::Duration};

use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ProfileId(String);

impl ProfileId {
    pub fn parse(value: impl Into<String>) -> Result<Self, AppError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
        if !valid {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "profile id must contain only ASCII letters, numbers, '-' or '_'",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ProfileId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandRisk {
    ReadOnly,
    LowRiskWrite,
    PrivilegedWrite,
    Destructive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandActor {
    UserInterface,
    Tray,
    Agent,
    Scheduler,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommandApproval {
    pub max_risk: CommandRisk,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommandContext {
    pub actor: CommandActor,
    pub approval: Option<CommandApproval>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppCommand {
    GetCoreNetworkSettings,
    UpdateCoreNetworkSettings {
        settings: CoreNetworkSettings,
    },
    GetRuntimeSettings,
    GetApplicationSettings,
    GetHelperStatus,
    /// 安装/修复特权 helper（持久化系统变更，需 macOS 管理员授权）。
    InstallHelper,
    /// 卸载特权 helper（移除 LaunchDaemon 与二进制，不可自动恢复）。
    UninstallHelper,
    UpdateApplicationSettings {
        settings: ApplicationSettings,
    },
    ExportApplicationSettings {
        destination: String,
    },
    PreviewApplicationSettingsImport {
        source: String,
    },
    ImportApplicationSettings {
        source: String,
    },
    ResetApplicationSettingsScope {
        scope: SettingsScope,
    },
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
    /// 检查应用自身更新（GitHub Releases），只读，不写任何系统状态。
    CheckAppUpdate,
    /// 下载、校验并替换当前 .app（高风险写入，UI 需确认；重启后生效）。
    UpdateApplication,
    /// 退出守护进程并以新安装的 .app 重启（高风险写入，UI 需确认）。
    RestartApplication,
    /// 完整退出应用：守护进程先清理系统状态，再结束 GUI 与托盘。
    QuitApplication,
    ListProfiles,
    GetProfileYaml {
        id: ProfileId,
    },
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
    SelectProfile {
        id: ProfileId,
    },
    DeleteProfile {
        id: ProfileId,
    },
    UpdateProfileYaml {
        id: ProfileId,
        yaml: String,
    },
    GetMergeConfig,
    UpdateMergeConfig {
        yaml: String,
    },
    GetMergedProfileYaml {
        id: ProfileId,
    },
    UpdateRemoteProfile {
        id: ProfileId,
    },
    SetProfileUpdatePolicy {
        id: ProfileId,
        update_policy: UpdatePolicy,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProfileSource {
    Local,
    Remote { url: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UpdatePolicy {
    Manual,
    Interval { seconds: u64 },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    pub source: ProfileSource,
    pub update_policy: UpdatePolicy,
    pub updated_at: i64,
    pub next_update_at: Option<i64>,
    #[serde(default)]
    pub consecutive_failures: u32,
    #[serde(default)]
    pub last_error: Option<String>,
    /// 订阅下载请求的自定义 User-Agent；None 使用默认 UA。
    #[serde(default)]
    pub user_agent: Option<String>,
}

impl Profile {
    pub fn new(
        id: ProfileId,
        name: impl Into<String>,
        source: ProfileSource,
        update_policy: UpdatePolicy,
        now: i64,
        user_agent: Option<String>,
    ) -> Result<Self, AppError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "profile name is empty",
            ));
        }
        if let ProfileSource::Remote { url } = &source
            && !(url.starts_with("https://") || url.starts_with("http://"))
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "remote profile URL must use HTTP or HTTPS",
            ));
        }
        let next_update_at = match &update_policy {
            UpdatePolicy::Manual => None,
            UpdatePolicy::Interval { seconds: 0 } => {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "update interval must be greater than zero",
                ));
            }
            UpdatePolicy::Interval { seconds } => {
                let seconds = i64::try_from(*seconds).map_err(|_| {
                    AppError::new(ErrorCode::InvalidInput, "update interval is too large")
                })?;
                Some(now.checked_add(seconds).ok_or_else(|| {
                    AppError::new(ErrorCode::InvalidInput, "next update time overflowed")
                })?)
            }
        };
        Ok(Self {
            id,
            name,
            source,
            update_policy,
            updated_at: now,
            next_update_at,
            consecutive_failures: 0,
            last_error: None,
            user_agent,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum AppCommandOutput {
    CoreNetworkSettings(CoreNetworkSettings),
    RuntimeSettings(RuntimeSettings),
    ApplicationSettings(ApplicationSettingsSnapshot),
    HelperStatus(HelperStatus),
    ApplicationSettingsExported {
        path: String,
    },
    ApplicationSettingsImportPreview(SettingsImportPreview),
    DiagnosticsExported {
        path: String,
    },
    EncryptedBackupExported {
        path: String,
    },
    MihomoUpdated {
        version: String,
    },
    AppUpdateStatus(AppUpdateStatus),
    ApplicationUpdated {
        version: String,
        restart_required: bool,
    },
    Profiles {
        profiles: Vec<Profile>,
        selected: Option<ProfileId>,
    },
    ProfileYaml {
        id: ProfileId,
        yaml: String,
    },
    MergeConfigYaml {
        yaml: String,
    },
    MergedProfileYaml {
        id: ProfileId,
        yaml: String,
    },
    None,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApplicationSettings {
    #[serde(default)]
    pub theme: ThemePreference,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default = "default_log_limit")]
    pub log_limit: u16,
    /// 登录时自动启动（macOS SMAppService 登录项）。
    #[serde(default)]
    pub launch_at_login: bool,
    /// “显示/隐藏主窗口”的全局快捷键（如 `CmdOrCtrl+Shift+V`），`None` 表示禁用。
    #[serde(default)]
    pub global_hotkey: Option<String>,
}

impl Default for ApplicationSettings {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            language: default_language(),
            log_limit: default_log_limit(),
            launch_at_login: false,
            global_hotkey: None,
        }
    }
}

impl ApplicationSettings {
    pub fn validate(&self) -> Result<(), AppError> {
        if !matches!(self.language.as_str(), "en" | "zh-CN") {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "language must be 'en' or 'zh-CN'",
            ));
        }
        if !(100..=5_000).contains(&self.log_limit) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "log limit must be between 100 and 5000",
            ));
        }
        if let Some(hotkey) = &self.global_hotkey {
            GlobalHotkeySpec::parse(hotkey)?;
        }
        Ok(())
    }

    /// 只把指定作用域的字段恢复为默认值,其它字段保持不变;
    /// Profile、备份等不在应用设置内的数据不受影响。
    pub fn reset_scope(&mut self, scope: SettingsScope) {
        let defaults = Self::default();
        match scope {
            SettingsScope::Appearance => {
                self.theme = defaults.theme;
                self.language = defaults.language;
            }
            // 应用设置当前没有持久化的网络字段(内核网络开关由运行时命令管理);
            // 该作用域保留给后续字段,重置是显式的空操作。
            SettingsScope::Network => {}
            SettingsScope::System => {
                self.log_limit = defaults.log_limit;
                self.launch_at_login = defaults.launch_at_login;
                self.global_hotkey = defaults.global_hotkey.clone();
            }
        }
    }
}

/// 全局快捷键的修饰键集合。`cmd_or_ctrl` 在 macOS 上是 Command，其它平台是 Ctrl。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HotkeyModifiers {
    pub cmd_or_ctrl: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// 全局快捷键可用的命名键（键名单大小写不敏感，归一化为这里的写法）。
/// 字母、数字和 F1–F12 不在此列，由解析器直接接受。
pub const GLOBAL_HOTKEY_NAMED_KEYS: &[&str] = &[
    "Space",
    "Tab",
    "Enter",
    "Escape",
    "Backspace",
    "Delete",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "Up",
    "Down",
    "Left",
    "Right",
    "Minus",
    "Equal",
    "Comma",
    "Period",
    "Slash",
    "Backslash",
    "Semicolon",
    "Quote",
    "Backquote",
    "BracketLeft",
    "BracketRight",
];

/// 解析校验过的全局快捷键：至少一个修饰键 + 规范化键名（如 `V`、`F5`、`Space`）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GlobalHotkeySpec {
    pub modifiers: HotkeyModifiers,
    pub key: String,
}

impl GlobalHotkeySpec {
    /// 解析 `CmdOrCtrl+Shift+V` 风格的快捷键字符串。大小写与常见别名
    /// （cmd/command/meta/super、ctrl/control、alt/option、return/esc 等）都会归一化；
    /// 没有修饰键、键名不在支持范围内或含控制字符都会被拒绝。
    pub fn parse(value: &str) -> Result<Self, AppError> {
        let invalid = |message: &str| AppError::new(ErrorCode::InvalidInput, message.to_owned());
        if value.trim() != value || value.chars().any(char::is_control) {
            return Err(invalid(
                "global hotkey must not have leading/trailing whitespace or control characters",
            ));
        }
        let tokens = value.split('+').map(str::trim).collect::<Vec<_>>();
        if tokens.iter().any(|token| token.is_empty()) {
            return Err(invalid("global hotkey contains an empty token"));
        }
        let Some((key_token, modifier_tokens)) = tokens.split_last() else {
            return Err(invalid("global hotkey is empty"));
        };
        let mut modifiers = HotkeyModifiers::default();
        for token in modifier_tokens {
            let slot = match token.to_ascii_lowercase().as_str() {
                "cmdorctrl" | "command" | "cmd" | "meta" | "super" => &mut modifiers.cmd_or_ctrl,
                "ctrl" | "control" => &mut modifiers.ctrl,
                "alt" | "option" => &mut modifiers.alt,
                "shift" => &mut modifiers.shift,
                _ => {
                    return Err(invalid(&format!(
                        "unsupported global hotkey modifier '{token}'"
                    )));
                }
            };
            if *slot {
                return Err(invalid(&format!(
                    "duplicate global hotkey modifier '{token}'"
                )));
            }
            *slot = true;
        }
        if modifiers == HotkeyModifiers::default() {
            return Err(invalid("global hotkey requires at least one modifier"));
        }
        let key = normalize_hotkey_key(key_token)
            .ok_or_else(|| invalid(&format!("unsupported global hotkey key '{key_token}'")))?;
        Ok(Self { modifiers, key })
    }

    /// 规范化字符串（修饰键固定顺序），用于比较和注册。
    pub fn normalized(&self) -> String {
        let mut parts = Vec::new();
        if self.modifiers.cmd_or_ctrl {
            parts.push("CmdOrCtrl");
        }
        if self.modifiers.ctrl {
            parts.push("Ctrl");
        }
        if self.modifiers.alt {
            parts.push("Alt");
        }
        if self.modifiers.shift {
            parts.push("Shift");
        }
        parts.push(&self.key);
        parts.join("+")
    }
}

fn normalize_hotkey_key(token: &str) -> Option<String> {
    if token.len() == 1 {
        let character = token.chars().next()?;
        return character
            .is_ascii_alphanumeric()
            .then(|| character.to_ascii_uppercase().to_string());
    }
    if let Some(number) = token
        .strip_prefix(['F', 'f'])
        .and_then(|rest| rest.parse::<u8>().ok())
        && (1..=12).contains(&number)
    {
        return Some(format!("F{number}"));
    }
    GLOBAL_HOTKEY_NAMED_KEYS
        .iter()
        .find(|name| name.eq_ignore_ascii_case(token))
        .or_else(|| match token.to_ascii_lowercase().as_str() {
            "return" => Some(&"Enter"),
            "esc" => Some(&"Escape"),
            "del" => Some(&"Delete"),
            _ => None,
        })
        .map(|name| (*name).to_owned())
}

/// 恢复默认值的作用域划分。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsScope {
    /// 外观:主题、语言。
    Appearance,
    /// 网络:预留给持久化的网络偏好。
    Network,
    /// 系统:日志缓冲、登录启动、全局快捷键。
    System,
}

/// 设置导入预览中的单字段差异。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SettingsFieldChange {
    pub field: String,
    pub old: String,
    pub new: String,
}

/// 设置导入前的差异预览:校验通过的待导入设置 + 逐字段 old→new 列表。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SettingsImportPreview {
    pub settings: ApplicationSettings,
    pub changes: Vec<SettingsFieldChange>,
}

fn default_language() -> String {
    "en".into()
}

const fn default_log_limit() -> u16 {
    500
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApplicationSettingsSnapshot {
    pub settings: ApplicationSettings,
    pub data_directory: String,
    /// 运行实例的版本（打包 .app 读 Info.plist；非打包运行为 None）。
    #[serde(default)]
    pub app_version: Option<String>,
}

/// 应用自身更新检查的结果快照。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppUpdateStatus {
    /// 当前运行实例版本；非 .app 运行（开发模式）为 None。
    pub current_version: Option<String>,
    /// GitHub Releases 最新正式版版本。
    pub latest_version: String,
    /// 最新版是否比当前版本新（无当前版本时恒为 false，禁止盲目更新）。
    pub update_available: bool,
    /// 发布资产下载地址（`Verge-macos-arm64.zip`，见 application::app_update 注释）。
    pub asset_url: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum HelperStatus {
    NotInstalled,
    Ready { protocol_version: u32 },
    Incompatible { message: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeSettings {
    pub system_proxy_services: Vec<String>,
    pub system_proxy_endpoint: ProxyEndpoint,
    pub system_proxy_socks_endpoint: Option<ProxyEndpoint>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppCommandResult {
    pub output: AppCommandOutput,
    pub summary: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RunMode {
    Rule,
    Global,
    Direct,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProxyGroup {
    pub name: String,
    pub kind: String,
    pub selected: Option<String>,
    pub members: Vec<String>,
}

/// One controller snapshot; node details are shared by all group occurrences.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProxySnapshot {
    pub groups: Vec<ProxyGroup>,
    pub proxies: std::collections::BTreeMap<String, ProxyDetails>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProxyDetails {
    pub kind: String,
    pub udp: Option<bool>,
    #[serde(default)]
    pub xudp: bool,
    #[serde(default)]
    pub tfo: bool,
    #[serde(default)]
    pub mptcp: bool,
    #[serde(default)]
    pub smux: bool,
    #[serde(default)]
    pub hidden: bool,
    pub selected: Option<String>,
    pub delay: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RuleEntry {
    pub kind: String,
    pub payload: String,
    pub proxy: String,
    pub size: i64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Proxy,
    Rule,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProviderSummary {
    pub name: String,
    pub kind: ProviderKind,
    pub vehicle: String,
    pub updated_at: String,
    pub item_count: usize,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct NetworkSettings {
    pub tun_enabled: bool,
    pub dns_enabled: bool,
    pub ipv6_enabled: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RealtimeTopic {
    Traffic,
    Memory,
    Connections,
    Logs,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TrafficEvent {
    pub up: u64,
    pub down: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MemoryEvent {
    pub inuse: u64,
    #[serde(default)]
    pub oslimit: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LogEvent {
    #[serde(rename = "type")]
    pub level: String,
    pub payload: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConnectionSnapshot {
    pub upload_total: u64,
    pub download_total: u64,
    pub connection_count: usize,
    #[serde(default)]
    pub connections: Vec<Connection>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Connection {
    pub id: String,
    pub network: String,
    pub source: String,
    pub destination: String,
    pub host: String,
    pub process: String,
    pub upload: u64,
    pub download: u64,
    pub start: String,
    pub rule: String,
    pub rule_payload: String,
    pub chains: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum RealtimeEvent {
    Traffic(TrafficEvent),
    Memory(MemoryEvent),
    Connections(ConnectionSnapshot),
    Log(LogEvent),
    Reconnecting { attempt: u32, delay: Duration },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeCommand {
    GetMode,
    SetMode {
        mode: RunMode,
    },
    ListProxyGroups,
    ListRules,
    ListProviders,
    GetNetworkSettings,
    SetNetworkSettings {
        settings: NetworkSettings,
    },
    UpdateProvider {
        kind: ProviderKind,
        name: String,
    },
    SelectProxy {
        group: String,
        proxy: String,
    },
    TestProxyDelay {
        proxy: String,
        url: String,
        timeout_ms: u32,
    },
    CloseConnection {
        id: String,
    },
    StartRealtime {
        topics: Vec<RealtimeTopic>,
    },
    StopRealtime,
    DrainRealtime,
}

impl RuntimeCommand {
    pub fn risk(&self) -> CommandRisk {
        match self {
            Self::GetMode
            | Self::ListProxyGroups
            | Self::ListRules
            | Self::ListProviders
            | Self::GetNetworkSettings
            | Self::StartRealtime { .. }
            | Self::StopRealtime
            | Self::DrainRealtime => CommandRisk::ReadOnly,
            Self::SelectProxy { .. }
            | Self::TestProxyDelay { .. }
            | Self::CloseConnection { .. }
            | Self::UpdateProvider { .. } => CommandRisk::LowRiskWrite,
            Self::SetMode { .. } | Self::SetNetworkSettings { .. } => CommandRisk::PrivilegedWrite,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum RuntimeCommandOutput {
    None,
    Mode(RunMode),
    ProxyGroups(ProxySnapshot),
    Rules(Vec<RuleEntry>),
    Providers(Vec<ProviderSummary>),
    NetworkSettings(NetworkSettings),
    Delay(u32),
    Realtime(Vec<RealtimeEvent>),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RuntimeCommandResult {
    pub output: RuntimeCommandOutput,
    pub summary: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProxyEndpoint {
    pub host: String,
    pub port: u16,
}

impl ProxyEndpoint {
    pub fn new(host: impl Into<String>, port: u16) -> Result<Self, AppError> {
        let host = host.into();
        if host.trim().is_empty() || host.chars().any(char::is_control) || port == 0 {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "proxy endpoint must have a valid host and non-zero port",
            ));
        }
        Ok(Self { host, port })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProxyProtocolState {
    pub enabled: bool,
    pub endpoint: ProxyEndpoint,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AutoProxyState {
    pub enabled: bool,
    pub url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SystemProxyServiceState {
    pub service: String,
    pub web: ProxyProtocolState,
    pub secure_web: ProxyProtocolState,
    pub socks: ProxyProtocolState,
    pub auto_proxy: AutoProxyState,
    pub bypass: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SystemProxyState {
    pub services: Vec<SystemProxyServiceState>,
    pub recovery_pending: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SystemProxyCommand {
    GetState,
    Enable {
        services: Vec<String>,
        endpoint: ProxyEndpoint,
    },
    Disable,
    RecoverPending,
    SetSocks {
        enabled: bool,
        endpoint: ProxyEndpoint,
    },
    SetAutoProxy {
        url: Option<String>,
    },
    SetProxyBypass {
        domains: Vec<String>,
    },
}

impl SystemProxyCommand {
    pub fn risk(&self) -> CommandRisk {
        match self {
            Self::GetState => CommandRisk::ReadOnly,
            Self::Enable { .. }
            | Self::Disable
            | Self::RecoverPending
            | Self::SetSocks { .. }
            | Self::SetAutoProxy { .. }
            | Self::SetProxyBypass { .. } => CommandRisk::PrivilegedWrite,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SystemProxyCommandResult {
    pub state: SystemProxyState,
    pub summary: String,
}

impl AppCommand {
    pub fn risk(&self) -> CommandRisk {
        match self {
            Self::GetCoreNetworkSettings
            | Self::GetRuntimeSettings
            | Self::GetApplicationSettings
            | Self::GetHelperStatus
            | Self::PreviewApplicationSettingsImport { .. }
            | Self::ListProfiles
            | Self::GetProfileYaml { .. }
            | Self::GetMergeConfig
            | Self::GetMergedProfileYaml { .. }
            | Self::CheckAppUpdate => CommandRisk::ReadOnly,
            Self::ImportProfile { .. }
            | Self::ImportRemoteProfile { .. }
            | Self::SelectProfile { .. }
            | Self::UpdateProfileYaml { .. }
            | Self::UpdateMergeConfig { .. }
            | Self::UpdateRemoteProfile { .. }
            | Self::SetProfileUpdatePolicy { .. }
            | Self::UpdateApplicationSettings { .. }
            | Self::ExportApplicationSettings { .. }
            | Self::ImportApplicationSettings { .. }
            | Self::ResetApplicationSettingsScope { .. }
            | Self::ExportDiagnostics { .. }
            | Self::ExportEncryptedBackup { .. } => CommandRisk::LowRiskWrite,
            Self::UpdateCoreNetworkSettings { .. }
            | Self::UpdateMihomo
            | Self::InstallHelper
            | Self::UpdateApplication
            | Self::RestartApplication => CommandRisk::PrivilegedWrite,
            Self::QuitApplication
            | Self::UninstallHelper
            | Self::DeleteProfile { .. }
            | Self::RestoreEncryptedBackup { .. } => CommandRisk::Destructive,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidInput,
    NotFound,
    Conflict,
    PermissionDenied,
    ValidationFailed,
    StorageFailed,
    PlatformFailed,
    CoreUnavailable,
    RequestTimeout,
    ProxyDelayFailed,
    CoreRejectedConfig,
    /// A newer peer may introduce codes without breaking the enclosing error response.
    /// Keep its message, but do not infer core health or recovery actions from an unknown code.
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl Error for AppError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_id_rejects_path_components() {
        assert_eq!(
            ProfileId::parse("../profile").unwrap_err().code,
            ErrorCode::InvalidInput
        );
        assert_eq!(
            SystemProxyCommand::Disable.risk(),
            CommandRisk::PrivilegedWrite
        );
    }

    #[test]
    fn commands_expose_risk_before_dispatch() {
        let command = AppCommand::DeleteProfile {
            id: ProfileId::parse("daily").unwrap(),
        };
        assert_eq!(command.risk(), CommandRisk::Destructive);
        // helper 安装是高权限写入（管理员授权），卸载是破坏性系统变更。
        assert_eq!(
            AppCommand::InstallHelper.risk(),
            CommandRisk::PrivilegedWrite
        );
        assert_eq!(AppCommand::UninstallHelper.risk(), CommandRisk::Destructive);
        // 应用自身更新：检查只读，替换与重启是高权限写入（UI 确认 + CommandBus 授权）。
        assert_eq!(AppCommand::CheckAppUpdate.risk(), CommandRisk::ReadOnly);
        assert_eq!(
            AppCommand::UpdateApplication.risk(),
            CommandRisk::PrivilegedWrite
        );
        assert_eq!(
            AppCommand::RestartApplication.risk(),
            CommandRisk::PrivilegedWrite
        );
        assert_eq!(AppCommand::QuitApplication.risk(), CommandRisk::Destructive);
        assert_eq!(AppCommand::GetHelperStatus.risk(), CommandRisk::ReadOnly);
        assert_eq!(
            RuntimeCommand::SetMode {
                mode: RunMode::Global,
            }
            .risk(),
            CommandRisk::PrivilegedWrite
        );
    }

    #[test]
    fn system_proxy_write_variants_are_privileged() {
        let endpoint = ProxyEndpoint::new("127.0.0.1", 7890).unwrap();
        for command in [
            SystemProxyCommand::SetSocks {
                enabled: true,
                endpoint,
            },
            SystemProxyCommand::SetAutoProxy {
                url: Some("http://127.0.0.1/proxy.pac".into()),
            },
            SystemProxyCommand::SetProxyBypass {
                domains: vec!["*.local".into()],
            },
        ] {
            assert_eq!(command.risk(), CommandRisk::PrivilegedWrite);
        }
    }

    #[test]
    fn merge_commands_expose_risk_before_dispatch() {
        assert_eq!(AppCommand::GetMergeConfig.risk(), CommandRisk::ReadOnly);
        assert_eq!(
            AppCommand::GetMergedProfileYaml {
                id: ProfileId::parse("daily").unwrap(),
            }
            .risk(),
            CommandRisk::ReadOnly
        );
        assert_eq!(
            AppCommand::UpdateMergeConfig {
                yaml: "rules: []\n".into(),
            }
            .risk(),
            CommandRisk::LowRiskWrite
        );
    }

    #[test]
    fn deserialization_cannot_bypass_profile_id_validation() {
        let error = serde_json::from_str::<ProfileId>(r#""../profile""#).unwrap_err();
        assert!(error.to_string().contains("profile id"));
    }

    #[test]
    fn settings_scope_reset_only_touches_its_own_fields() {
        let mut settings = ApplicationSettings {
            theme: ThemePreference::Dark,
            language: "zh-CN".into(),
            log_limit: 1_000,
            ..ApplicationSettings::default()
        };
        settings.reset_scope(SettingsScope::Appearance);
        assert_eq!(settings.theme, ThemePreference::System);
        assert_eq!(settings.language, "en");
        assert_eq!(settings.log_limit, 1_000);

        settings.reset_scope(SettingsScope::Network);
        assert_eq!(settings.log_limit, 1_000);

        settings.reset_scope(SettingsScope::System);
        assert_eq!(settings.log_limit, 500);
        assert_eq!(settings.theme, ThemePreference::System);
    }

    #[test]
    fn system_scope_reset_clears_launch_at_login_and_global_hotkey() {
        let mut settings = ApplicationSettings {
            launch_at_login: true,
            global_hotkey: Some("CmdOrCtrl+Shift+V".into()),
            log_limit: 1_000,
            ..ApplicationSettings::default()
        };
        settings.reset_scope(SettingsScope::System);
        assert!(!settings.launch_at_login);
        assert_eq!(settings.global_hotkey, None);
    }

    #[test]
    fn global_hotkey_parse_normalizes_aliases_and_order() {
        let spec = GlobalHotkeySpec::parse("shift+command+v").unwrap();
        assert!(spec.modifiers.cmd_or_ctrl);
        assert!(spec.modifiers.shift);
        assert_eq!(spec.key, "V");
        assert_eq!(spec.normalized(), "CmdOrCtrl+Shift+V");
        assert_eq!(
            GlobalHotkeySpec::parse("Option+F5").unwrap().normalized(),
            "Alt+F5"
        );
        assert_eq!(
            GlobalHotkeySpec::parse("Control+Return")
                .unwrap()
                .normalized(),
            "Ctrl+Enter"
        );
        // 键名只能放在最后一个 token。
        assert_eq!(
            GlobalHotkeySpec::parse("V+Cmd+Shift").unwrap_err().code,
            ErrorCode::InvalidInput
        );
        // 相同快捷键的不同写法归一化后相等。
        assert_eq!(
            GlobalHotkeySpec::parse("Shift+Meta+Space").unwrap(),
            GlobalHotkeySpec::parse("shift+cmdorctrl+space").unwrap()
        );
    }

    #[test]
    fn global_hotkey_parse_rejects_invalid_input() {
        for value in [
            "",
            "   ",
            "CmdOrCtrl+",
            "+Shift+V",
            "CmdOrCtrl++V",
            "V",
            "Shift+Space+x",
            "CmdOrCtrl+Cmd+1",
            "CmdOrCtrl+F13",
            "CmdOrCtrl+SideButton",
            " CmdOrCtrl+V",
            "CmdOrCtrl+\nv",
        ] {
            assert_eq!(
                GlobalHotkeySpec::parse(value).unwrap_err().code,
                ErrorCode::InvalidInput,
                "value: {value:?}"
            );
        }
    }

    #[test]
    fn settings_validation_applies_global_hotkey_rules() {
        let mut settings = ApplicationSettings {
            global_hotkey: Some("CmdOrCtrl+Shift+V".into()),
            ..ApplicationSettings::default()
        };
        settings.validate().unwrap();
        settings.global_hotkey = Some("not-a-hotkey".into());
        assert_eq!(
            settings.validate().unwrap_err().code,
            ErrorCode::InvalidInput
        );
        settings.global_hotkey = None;
        settings.validate().unwrap();
    }

    #[test]
    fn settings_transfer_commands_expose_risk_before_dispatch() {
        assert_eq!(
            AppCommand::PreviewApplicationSettingsImport {
                source: "/tmp/settings.json".into(),
            }
            .risk(),
            CommandRisk::ReadOnly
        );
        for command in [
            AppCommand::ExportApplicationSettings {
                destination: "/tmp/settings.json".into(),
            },
            AppCommand::ImportApplicationSettings {
                source: "/tmp/settings.json".into(),
            },
            AppCommand::ResetApplicationSettingsScope {
                scope: SettingsScope::Appearance,
            },
        ] {
            assert_eq!(command.risk(), CommandRisk::LowRiskWrite);
        }
    }
}
