use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default)]
    pub id: String,

    #[serde(default)]
    pub name: String,

    #[serde(default)]
    pub file_name: String,

    /// 文件格式 clash-yaml | json | ...
    #[serde(default)]
    pub format: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub subsciption: Option<SubscriptionInfo>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<usize>,
}

#[derive(Default, Debug, Clone, Copy, Deserialize, Serialize)]
pub struct SubscriptionInfo {
    pub upload: usize,
    pub download: usize,
    pub total: usize,
    pub expire: usize,
}
