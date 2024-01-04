use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct RuleSet {
    pub tag: String,
    pub format: String,
    #[serde(flatten)]
    pub option: RuleSetOptions,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RuleSetOptions {
    #[serde(rename = "local")]
    Local(LocalRuleSet),
    #[serde(rename = "remote")]
    Remote(RemoteRuleSet),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LocalRuleSet {
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RemoteRuleSet {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_detour: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_interval: Option<String>,
}
