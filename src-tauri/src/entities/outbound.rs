use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::common::*;

#[derive(Debug, Serialize, Deserialize)]
pub struct Outbound {
    pub tag: String,
    #[serde(flatten)]
    pub option: OutboundOptions,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OutboundOptions {
    #[serde(rename = "dns")]
    DNS,
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
    VMess(VMessOptions),
    #[serde(rename = "trojan")]
    Trojan(TrojanOptions),
    #[serde(rename = "wireguard")]
    WireGuard(WireGuardOptions),
    #[serde(rename = "hysteria")]
    Hysteria(HysteriaOptions),
    #[serde(rename = "tor")]
    Tor(TorOptions),
    #[serde(rename = "ssh")]
    SSH(SSHOptions),
    #[serde(rename = "shadowsocksr")]
    ShadowsocksR(ShadowsocksROptions),
    #[serde(rename = "shadowtls")]
    ShadowTLS(ShadowTLSOptions),
    #[serde(rename = "vless")]
    VLESS(VLESSOptions),
    #[serde(rename = "tuic")]
    TUIC(TUICOptions),
    #[serde(rename = "hysteria2")]
    Hysteria2(Hysteria2Options),
    #[serde(rename = "selector")]
    Selector(SelectorOptions),
    #[serde(rename = "urltest")]
    URLTest(URLTestOptions),
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
    pub plugin_opts: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udp_over_tcp: Option<UDPOverTCPOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiplex: Option<OutboundMultiplexOptions>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct VMessOptions {
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

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct TrojanOptions {
    #[serde(flatten)]
    pub dialer_options: DialerOptions,
    #[serde(flatten)]
    pub server_options: ServerOptions,
    pub password: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<OutboundTLSOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiplex: Option<OutboundMultiplexOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<V2RayTransportOptions>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct WireGuardOptions {
    #[serde(flatten)]
    pub dialer_options: DialerOptions,
    #[serde(flatten)]
    pub server_options: ServerOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_interface: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gso: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gso_max_size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interface_name: Option<String>,
    pub local_address: Vec<String>,
    pub private_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peers: Option<Vec<WireGuardPeer>>,
    pub peer_public_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_shared_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reserved: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workers: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtu: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct HysteriaOptions {
    #[serde(flatten)]
    pub dialer_options: DialerOptions,
    #[serde(flatten)]
    pub server_options: ServerOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub up: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub up_mbps: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub down: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub down_mbps: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obfs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_str: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recv_window_conn: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recv_window: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_mtu_discovery: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<OutboundTLSOptions>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct TorOptions {
    #[serde(flatten)]
    pub dialer_options: DialerOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_args: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_directory: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct SSHOptions {
    #[serde(flatten)]
    pub dialer_options: DialerOptions,
    #[serde(flatten)]
    pub server_options: ServerOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_key: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_key_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub private_key_passphrase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_key: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_key_algorithms: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_version: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ShadowTLSOptions {
    #[serde(flatten)]
    pub dialer_options: DialerOptions,
    #[serde(flatten)]
    pub server_options: ServerOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<i32>,
    pub password: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<OutboundTLSOptions>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ShadowsocksROptions {
    #[serde(flatten)]
    pub dialer_options: DialerOptions,
    #[serde(flatten)]
    pub server_options: ServerOptions,
    pub method: String,
    pub password: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obfs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obfs_param: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_param: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct VLESSOptions {
    #[serde(flatten)]
    pub dialer_options: DialerOptions,
    #[serde(flatten)]
    pub server_options: ServerOptions,
    pub uuid: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<OutboundTLSOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiplex: Option<OutboundMultiplexOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<V2RayTransportOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packet_encoding: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct TUICOptions {
    #[serde(flatten)]
    pub dialer_options: DialerOptions,
    #[serde(flatten)]
    pub server_options: ServerOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub congestion_control: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub udp_relay_mode: Option<String>,
    pub udp_over_stream: bool,
    pub zero_rtt_handshake: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heartbeat: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<OutboundTLSOptions>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Hysteria2Options {
    #[serde(flatten)]
    pub dialer_options: DialerOptions,
    #[serde(flatten)]
    pub server_options: ServerOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub up_mbps: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub down_mbps: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obfs: Option<Hysteria2Obfs>,
    pub password: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<OutboundTLSOptions>,
    pub brutal_debug: bool,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct SelectorOptions {
    pub outbounds: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interrupt_exist_connections: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct URLTestOptions {
    pub outbounds: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tolerance: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interrupt_exist_connections: Option<bool>,
}
