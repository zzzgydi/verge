use super::{InboundOptions, ListenOptions, TunPlatformOptions, UserOptions};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Inbound {
    pub tag: String,
    #[serde(flatten)]
    pub option: InboundType,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum InboundType {
    #[serde(rename = "mixed")]
    Mixed(MixedInbound),
    #[serde(rename = "tun")]
    Tun(TunInbound),
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct MixedInbound {
    #[serde(flatten)]
    pub listen_options: ListenOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub users: Option<Vec<UserOptions>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub set_system_proxy: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct TunInbound {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interface_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtu: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inet4_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inet6_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_route: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict_route: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inet4_route_address: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inet6_route_address: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inet4_route_exclude_address: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inet6_route_exclude_address: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_interface: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_interface: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_uid: Option<Vec<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_uid_range: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_uid: Option<Vec<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_uid_range: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_android_user: Option<Vec<i32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_package: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_package: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_independent_nat: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udp_timeout: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stack: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<TunPlatformOptions>,
    #[serde(flatten)]
    pub inbound_options: InboundOptions,
}
