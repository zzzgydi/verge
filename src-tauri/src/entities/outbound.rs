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
pub enum OutboundOptions {
    Direct(DirectOptions),
    Block,
    Socks(SocksOptions),
    HTTP(HTTPOptions),
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
