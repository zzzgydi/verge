use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct InboundOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sniff: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sniff_override_destination: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sniff_timeout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udp_disable_domain_unmapping: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ListenOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listen_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tcp_fast_open: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tcp_multi_path: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udp_fragment: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udp_timeout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detour: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sniff: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sniff_override_destination: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sniff_timeout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udp_disable_domain_unmapping: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct UserOptions {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct TunPlatformOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_proxy: Option<HTTPProxyOptions>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct HTTPProxyOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    pub server: String,
    pub server_port: u16,
}
