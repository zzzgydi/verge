use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DNSRule {
    #[serde(rename = "default")]
    Default(DefaultDNSRule),
    #[serde(rename = "logical")]
    Logical(LogicalDNSRule),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DefaultDNSRule {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inbound: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip_version: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query_type: Option<Vec<u16>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_user: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_suffix: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_keyword: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_regex: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geosite: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_geoip: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ip_cidr: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ip_is_private: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_port: Option<Vec<u16>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_port_range: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<Vec<u16>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port_range: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_name: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_path: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_name: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Vec<i32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outbound: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clash_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wifi_ssid: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wifi_bssid: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_set: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invert: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_cache: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rewrite_ttl: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LogicalDNSRule {
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules: Option<Vec<DNSRule>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invert: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_cache: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rewrite_ttl: Option<u32>,
}
