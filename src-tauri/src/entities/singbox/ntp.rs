use serde::{Deserialize, Serialize};

use super::DialerOptions;

#[derive(Debug, Serialize, Deserialize)]
pub struct NTPOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_to_system: Option<bool>,
    #[serde(flatten)]
    pub dialer_options: DialerOptions,
}
