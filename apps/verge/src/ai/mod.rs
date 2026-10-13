//! A bounded diagnostic assistant with separately confirmed proposals. Network and configuration work run off the daemon loop.
pub mod diagnostics;
pub mod proposals;
mod provider;
mod service;
mod settings;
pub mod tools;
pub use service::AiService;
pub use settings::ProviderConfig;

use crate::domain::{AppError, ErrorCode};
use serde::{Deserialize, Serialize};

pub const CAPABILITY: &str = "ai_chat_v1";
pub const UX_CAPABILITY: &str = "ai_chat_ux_v2";
pub const ACTIONS_CAPABILITY: &str = "ai_actions_v1";
pub const TEXT_LIMIT: usize = 32 * 1024;

pub(crate) fn explain_health(
    directory: &std::path::Path,
    evidence: Vec<tools::Evidence>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<String, AppError> {
    let saved = settings::load(directory)?;
    let mut config = saved.config.validated()?;
    config.max_tool_steps = 0;
    config.timeout_seconds = config.timeout_seconds.min(30);
    let chinese = crate::config::FileSettingsStore::open(directory)
        .is_ok_and(|settings| settings.get().language == "zh-CN");
    let prompt = format!(
        "Explain this local network check concisely in {}. Only use the supplied report. Separate observations from hypotheses; list one next check. No tools or actions are available. Never claim browser connectivity was verified. Any recorded switch outcome refers only to the reported node probes.",
        if chinese { "Chinese" } else { "English" }
    );
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| error("Cannot start analysis"))?;
    runtime.block_on(provider::run_with_session(
        config,
        saved.api_key,
        vec![ChatMessage {
            role: "user".into(),
            text: prompt,
            evidence: Vec::new(),
        }],
        evidence,
        false,
        cancel,
        |_, _| {},
        None,
    ))
}

#[derive(Clone, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Secret(pub String);

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AiCommand {
    GetState,
    SaveConfig {
        config: ProviderConfig,
        api_key: Secret,
        clear_key: bool,
    },
    TestProvider,
    Retry,
    Start {
        prompt: String,
    },
    Cancel,
    Clear,
    Approve {
        id: String,
        digest: String,
    },
    Dismiss {
        id: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AiOperation {
    State,
    Save,
    Test,
    Retry,
    Start,
    Cancel,
    Clear,
    Approve,
    Dismiss,
}

impl AiCommand {
    pub fn operation(&self) -> AiOperation {
        match self {
            Self::GetState => AiOperation::State,
            Self::SaveConfig { .. } => AiOperation::Save,
            Self::TestProvider => AiOperation::Test,
            Self::Retry => AiOperation::Retry,
            Self::Start { .. } => AiOperation::Start,
            Self::Cancel => AiOperation::Cancel,
            Self::Clear => AiOperation::Clear,
            Self::Approve { .. } => AiOperation::Approve,
            Self::Dismiss { .. } => AiOperation::Dismiss,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub text: String,
    #[serde(default)]
    pub evidence: Vec<tools::Evidence>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AiSnapshot {
    pub revision: u64,
    pub run_id: u64,
    pub config: ProviderConfig,
    pub has_key: bool,
    pub busy: bool,
    #[serde(default)]
    pub operation: Option<AiOperation>,
    pub messages: Vec<ChatMessage>,
    pub activity: String,
    pub error: Option<String>,
    pub evidence: Vec<tools::Evidence>,
    #[serde(default)]
    pub proposals: Vec<proposals::Proposal>,
}

pub(crate) fn error(message: &str) -> AppError {
    AppError::new(ErrorCode::InvalidInput, message)
}

pub(crate) fn bounded(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;
    #[test]
    fn snapshots_before_ux_fields_remain_readable() {
        let value = serde_json::json!({"revision":1,"run_id":1,"config":ProviderConfig::default(),"has_key":false,"busy":false,"messages":[{"role":"user","text":"question"}],"activity":"Completed","error":null,"evidence":[]});
        let state: AiSnapshot = serde_json::from_value(value).unwrap();
        assert_eq!(state.operation, None);
        assert!(state.proposals.is_empty());
        assert!(state.messages[0].evidence.is_empty());
    }
}
