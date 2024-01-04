use serde::{Deserialize, Serialize};

use super::{Rule, RuleSet};

#[derive(Debug, Serialize, Deserialize)]
pub struct RouteOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geoip: Option<GeoIPOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub geosite: Option<GeositeOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rules: Option<Vec<Rule>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_set: Option<Vec<RuleSet>>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "final")]
    pub r#final: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub find_process: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_detect_interface: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub override_android_vpn: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_interface: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_mark: Option<i32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GeoIPOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_detour: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GeositeOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_detour: Option<String>,
}
