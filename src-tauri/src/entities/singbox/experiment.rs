use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ExperimentalOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_file: Option<CacheFileOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clash_api: Option<ClashAPIOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub v2ray_api: Option<V2RayAPIOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug: Option<DebugOptions>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CacheFileOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_fakeip: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClashAPIOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_controller: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_ui: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_ui_download_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_ui_download_detour: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_mode: Option<String>,

    #[deprecated]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_file: Option<String>,
    #[deprecated]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_id: Option<String>,
    #[deprecated]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_mode: Option<bool>,
    #[deprecated]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_selected: Option<bool>,
    #[deprecated]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_fakeip: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct V2RayAPIOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<V2RayStatsServiceOptions>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct V2RayStatsServiceOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub inbounds: Vec<String>,
    #[serde(default)]
    pub outbounds: Vec<String>,
    #[serde(default)]
    pub users: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DebugOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listen: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gc_percent: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_stack: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_threads: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub panic_on_fault: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_back: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oom_killer: Option<bool>,
}
