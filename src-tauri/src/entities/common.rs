use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct DialerOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detour: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bind_interface: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inet4_bind_address: Option<IpAddr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inet6_bind_address: Option<IpAddr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protect_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_mark: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reuse_addr: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connect_timeout: Option<Duration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tcp_fast_open: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tcp_multi_path: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udp_fragment: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_strategy: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_delay: Option<Duration>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ServerOptions {
    pub server: String,
    pub server_port: u16,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct UDPOverTCPOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<u8>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct OutboundTLSOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_sni: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insecure: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alpn: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cipher_suites: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub certificate_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ech: Option<OutboundECHOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utls: Option<OutboundUTLSOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reality: Option<OutboundRealityOptions>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct OutboundECHOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pq_signature_schemes_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_record_sizing_disabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct OutboundUTLSOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct OutboundRealityOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_id: Option<String>,
}
