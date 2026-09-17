//! A bounded, read-only assistant. Network and Keychain work run off the daemon loop.
mod provider;
mod service;
mod settings;
pub mod tools;
pub use service::AiService;
pub use settings::ProviderConfig;

use crate::domain::{AppError, ErrorCode};
use serde::{Deserialize, Serialize};

pub const CAPABILITY: &str = "ai_chat_v1";
pub const TEXT_LIMIT: usize = 32 * 1024;

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
    Start {
        prompt: String,
    },
    Cancel,
    Clear,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AiOperation {
    State,
    Save,
    Test,
    Start,
    Cancel,
    Clear,
}

impl AiCommand {
    pub fn operation(&self) -> AiOperation {
        match self {
            Self::GetState => AiOperation::State,
            Self::SaveConfig { .. } => AiOperation::Save,
            Self::TestProvider => AiOperation::Test,
            Self::Start { .. } => AiOperation::Start,
            Self::Cancel => AiOperation::Cancel,
            Self::Clear => AiOperation::Clear,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub text: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AiSnapshot {
    pub revision: u64,
    pub run_id: u64,
    pub config: ProviderConfig,
    pub has_key: bool,
    pub busy: bool,
    pub messages: Vec<ChatMessage>,
    pub activity: String,
    pub error: Option<String>,
    pub evidence: Vec<tools::Evidence>,
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
