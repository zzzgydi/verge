use super::singbox::Outbound;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ProxyItem {
    pub pid: String, // profile id
    pub id: String,  // proxy item id
    pub outbound: Outbound,
    pub disabled: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProxyGroup {
    pub id: String,
    pub name: String,
    pub proxies: Vec<String>, // proxy item id list
    pub option: ProxyGroupOption,
    pub disabled: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ProxyGroupOption {
    #[serde(rename = "selector")]
    Selector(PgSelector),
    #[serde(rename = "urltest")]
    URLTest(PgURLTest),
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct PgSelector {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interrupt_exist_connections: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct PgURLTest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tolerance: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interrupt_exist_connections: Option<bool>,
}
