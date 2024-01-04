use serde::{Deserialize, Serialize};

use super::{DNSOptions, ExperimentalOptions, Inbound, NTPOptions, Outbound, RouteOptions};

#[derive(Debug, Serialize, Deserialize)]
pub struct SBConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log: Option<LogOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dns: Option<DNSOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ntp: Option<NTPOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inbounds: Option<Vec<Inbound>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outbounds: Option<Vec<Outbound>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<RouteOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub experimental: Option<ExperimentalOptions>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LogOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<bool>,
}
