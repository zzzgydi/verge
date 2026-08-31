use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_REQUEST_BYTES: u64 = 16 * 1024;

/// TUN 生命周期请求参数。helper 侧负责逐字段校验；这里只做传输。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TunConfig {
    /// 期望的设备名（`utunN`）；None 表示由 helper 分配空闲单元。
    #[serde(default)]
    pub device: Option<String>,
    /// 设备 IPv4 地址（点分十进制）。
    pub address: String,
    /// IPv4 网络掩码（点分十进制）。
    pub netmask: String,
    pub mtu: u16,
    /// 需要挂到设备上的路由表项（CIDR，如 `198.18.0.0/16`）。
    #[serde(default)]
    pub routes: Vec<String>,
}

impl TunConfig {
    /// Verge 默认的 TUN 参数：与 Mihomo 默认 TUN 地址段保持一致。
    pub fn verge_default() -> Self {
        Self {
            device: None,
            address: "198.18.0.1".into(),
            netmask: "255.255.0.0".into(),
            mtu: 9_000,
            routes: vec!["198.18.0.0/16".into()],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HelperRequest {
    Ping { protocol_version: u32 },
    GetCapabilities,
    EnableTun { config: TunConfig },
    /// device 为 None 时拆除 helper 当前管理的全部 TUN 设备。
    DisableTun { device: Option<String> },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HelperResponse {
    Pong {
        protocol_version: u32,
    },
    Capabilities {
        protocol_version: u32,
        tun_lifecycle: bool,
    },
    /// TUN 已就绪，返回实际设备名（如 `utun4`）。
    TunEnabled {
        device: String,
    },
    TunDisabled,
    Error {
        code: String,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tun_requests_and_responses_round_trip() {
        let requests = [
            HelperRequest::EnableTun {
                config: TunConfig::verge_default(),
            },
            HelperRequest::EnableTun {
                config: TunConfig {
                    device: Some("utun7".into()),
                    ..TunConfig::verge_default()
                },
            },
            HelperRequest::DisableTun {
                device: Some("utun7".into()),
            },
            HelperRequest::DisableTun { device: None },
        ];
        for request in requests {
            let bytes = serde_json::to_vec(&request).unwrap();
            assert_eq!(serde_json::from_slice::<HelperRequest>(&bytes).unwrap(), request);
        }
        let responses = [
            HelperResponse::TunEnabled {
                device: "utun4".into(),
            },
            HelperResponse::TunDisabled,
            HelperResponse::Error {
                code: "invalid_tun_config".into(),
                message: "bad device".into(),
            },
        ];
        for response in responses {
            let bytes = serde_json::to_vec(&response).unwrap();
            assert_eq!(
                serde_json::from_slice::<HelperResponse>(&bytes).unwrap(),
                response
            );
        }
    }

    #[test]
    fn verge_default_matches_mihomo_tun_defaults() {
        let config = TunConfig::verge_default();
        assert_eq!(config.device, None);
        assert_eq!(config.address, "198.18.0.1");
        assert_eq!(config.netmask, "255.255.0.0");
        assert!(config.mtu > 0);
        assert_eq!(config.routes, ["198.18.0.0/16"]);
    }
}
