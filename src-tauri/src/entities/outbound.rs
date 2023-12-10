use serde::{Deserialize, Serialize};
use std::{collections::HashMap, time::Duration};

use super::common::*;

#[derive(Debug, Serialize, Deserialize)]
pub struct Outbound {
    pub r#type: String,
    pub tag: String,

    #[serde(flatten)]
    pub option: OutboundOptions,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OutboundOptions {
    #[serde(rename = "direct")]
    Direct(DirectOptions),
    #[serde(rename = "block")]
    Block,
    #[serde(rename = "socks")]
    Socks(SocksOptions),
    #[serde(rename = "http")]
    HTTP(HTTPOptions),
    #[serde(rename = "shadowsocks")]
    Shadowsocks(ShadowsocksOptions),
    #[serde(rename = "vmess")]
    VMess(VMessOutboundOptions),
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct DirectOptions {
    #[serde(flatten)]
    pub dialer_options: DialerOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub override_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub override_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_protocol: Option<u8>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct SocksOptions {
    #[serde(flatten)]
    pub dialer_options: DialerOptions,
    #[serde(flatten)]
    pub server_options: ServerOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udp_over_tcp: Option<UDPOverTCPOptions>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct HTTPOptions {
    #[serde(flatten)]
    pub dialer_options: DialerOptions,
    #[serde(flatten)]
    pub server_options: ServerOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<OutboundTLSOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, Vec<String>>>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ShadowsocksOptions {
    #[serde(flatten)]
    pub dialer_options: DialerOptions,
    #[serde(flatten)]
    pub server_options: ServerOptions,
    pub method: String,
    pub password: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin_options: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udp_over_tcp: Option<UDPOverTCPOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiplex: Option<OutboundMultiplexOptions>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct VMessOutboundOptions {
    #[serde(flatten)]
    pub dialer_options: DialerOptions,
    #[serde(flatten)]
    pub server_options: ServerOptions,
    pub uuid: String,
    pub security: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alter_id: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_padding: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authenticated_length: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<OutboundTLSOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packet_encoding: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiplex: Option<OutboundMultiplexOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<V2RayTransportOptions>,
}
