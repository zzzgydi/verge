use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;

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
    pub connect_timeout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tcp_fast_open: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tcp_multi_path: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udp_fragment: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_strategy: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback_delay: Option<String>,
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

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct OutboundMultiplexOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_connections: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_streams: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_streams: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub padding: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brutal: Option<BrutalOptions>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct BrutalOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub up_mbps: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub down_mbps: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum V2RayTransportOptions {
    #[serde(rename = "http")]
    HTTP(V2RayHTTPOptions),
    #[serde(rename = "ws")]
    Websocket(V2RayWebsocketOptions),
    #[serde(rename = "quic")]
    QUIC,
    #[serde(rename = "grpc")]
    GRPC(V2RayGRPCOptions),
    #[serde(rename = "httpupgrade")]
    HTTPUpgrade(V2RayHTTPUpgradeOptions),
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(tag = "type")]
pub struct V2RayHTTPOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, Vec<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_timeout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ping_timeout: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct V2RayWebsocketOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, Vec<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_early_data: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub early_data_header_name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct V2RayGRPCOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_timeout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ping_timeout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permit_without_stream: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct V2RayHTTPUpgradeOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, Vec<String>>>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct WireGuardPeer {
    #[serde(flatten)]
    pub server_options: ServerOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_shared_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed_ips: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reserved: Option<Vec<u8>>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Hysteria2Obfs {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}
