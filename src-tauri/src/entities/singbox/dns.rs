use serde::{Deserialize, Serialize};

use super::DNSRule;

#[derive(Debug, Serialize, Deserialize)]
pub struct DNSOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub servers: Option<Vec<DNSServerOptions>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules: Option<Vec<DNSRule>>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "final")]
    pub r#final: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reverse_mapping: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fakeip: Option<DNSFakeIPOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_cache: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disable_expire: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub independent_cache: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DNSServerOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address_resolver: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address_strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detour: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DNSFakeIPOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inet4_range: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inet6_range: Option<String>,
}
