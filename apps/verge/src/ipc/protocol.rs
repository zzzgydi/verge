//! 守护进程与 GUI 进程之间的消息协议。
//!
//! 复用 `ui` 模块的 `UiRequestEnvelope` / `UiResponseEnvelope`：请求与响应语义与
//! 单进程时代完全一致，IPC 只换传输层。新增的消息只负责连接生命周期（握手、
//! 订阅、窗口激活、重复实例）与批量实时事件转发。

use crate::domain::{
    ApplicationSettingsSnapshot, Profile, ProfileId, RealtimeEvent, RuntimeSettings,
};
use crate::ui::{UiRequestEnvelope, UiResponseEnvelope};
use serde::{Deserialize, Serialize};

/// IPC 协议版本。守护进程与 GUI 进程必须一致；升级不匹配时拒绝连接。
/// v5: proxy snapshots include shared node details and capabilities.
/// 应用更新后旧守护进程与新 GUI 的配对靠这个版本号明确拒绝。
pub const PROTOCOL_VERSION: u32 = 5;

/// GUI → Daemon 的消息。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum DaemonMessage {
    /// 连接建立后的第一帧必须是 Hello。
    Hello {
        protocol_version: u32,
        app_version: String,
    },
    /// 常规请求，语义与单进程时代 `UiRequestEnvelope` 完全一致。
    /// 实时订阅复用 `RuntimeCommand::StartRealtime` / `StopRealtime`，
    /// 守护进程在处理它们时同步维护按连接的订阅表。
    Request(UiRequestEnvelope),
}

/// Daemon → GUI 的消息。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ClientMessage {
    /// 握手成功；`initial` 是一次拉全的领域状态快照，GUI 用于首帧渲染。
    Welcome {
        protocol_version: u32,
        initial: InitialSnapshot,
    },
    /// 请求响应，语义与单进程时代 `UiResponseEnvelope` 完全一致。
    Response(UiResponseEnvelope),
    /// 聚合后的实时事件批量（100ms 一帧），避免逐条推送撑爆 socket。
    RealtimeBatch(Vec<RealtimeEvent>),
    /// 唤醒已有窗口（守护进程要求 GUI 显示/激活主窗口）。
    ActivateWindow,
    /// 隐藏主窗口。
    HideWindow,
    /// 已有 GUI 主实例在运行，本实例应退出（守护进程随后会发 ActivateWindow 给旧实例）。
    Duplicate,
    /// 服务端主动关闭连接（版本不匹配等），GUI 应提示后退出。
    Closed { reason: String },
}

/// 连接握手成功后一次拉全的领域状态快照。
///
/// 只包含可重建的领域态；会话态（导航、表单草稿、抽屉内容）由 GUI 进程持有，
/// 每次拉起允许丢失。核心状态与实时数据仍由后续请求与订阅增量补齐。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InitialSnapshot {
    pub profiles: Vec<Profile>,
    pub selected_profile: Option<ProfileId>,
    pub application_settings: ApplicationSettingsSnapshot,
    pub runtime_settings: Option<RuntimeSettings>,
}

impl Default for InitialSnapshot {
    fn default() -> Self {
        Self {
            profiles: Vec::new(),
            selected_profile: None,
            application_settings: ApplicationSettingsSnapshot {
                settings: Default::default(),
                data_directory: String::new(),
                app_version: None,
            },
            runtime_settings: None,
        }
    }
}

impl InitialSnapshot {
    /// 快照是否为空（守护进程尚未就绪时可能为空，GUI 据此决定是否走全量刷新）。
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
            && self.selected_profile.is_none()
            && self.runtime_settings.is_none()
    }
}
