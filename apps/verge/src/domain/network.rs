use super::{AppError, ErrorCode};
use serde::{Deserialize, Serialize};

/// User overrides are separate from subscription YAML and temporary TUN state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CoreNetworkSettings {
    pub allow_lan: bool,
    pub ipv6: bool,
    pub unified_delay: bool,
    pub log_level: String,
    pub mixed_port: u16,
    pub dns_override: bool,
    pub dns_yaml: String,
    pub external_controller: ExternalControllerSettings,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExternalControllerSettings {
    pub enabled: bool,
    pub address: String,
    pub secret: String,
}
impl std::fmt::Debug for ExternalControllerSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExternalControllerSettings")
            .field("enabled", &self.enabled)
            .field("address", &self.address)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}
impl Default for ExternalControllerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            address: "127.0.0.1:9097".into(),
            secret: String::new(),
        }
    }
}
impl Default for CoreNetworkSettings {
    fn default() -> Self {
        Self {
            allow_lan: false, ipv6: false, unified_delay: true,
            log_level: "info".into(), mixed_port: 7897, dns_override: false,
            dns_yaml: "enable: true\nenhanced-mode: fake-ip\nfake-ip-range: 198.18.0.1/16\nnameserver:\n  - https://1.1.1.1/dns-query\n  - https://dns.alidns.com/dns-query\n".into(),
            external_controller: ExternalControllerSettings::default(),
        }
    }
}
impl CoreNetworkSettings {
    pub fn validate(&self) -> Result<(), AppError> {
        let invalid = |message| AppError::new(ErrorCode::ValidationFailed, message);
        if self.mixed_port == 0 {
            return Err(invalid("代理端口必须在 1–65535 之间"));
        }
        if !["silent", "error", "warning", "info", "debug"].contains(&self.log_level.as_str()) {
            return Err(invalid("日志等级无效"));
        }
        if self.dns_yaml.len() > 64 * 1024 {
            return Err(invalid("DNS 配置不能超过 64 KiB"));
        }
        let dns: serde_yaml::Value = serde_yaml::from_str(&self.dns_yaml)
            .map_err(|_| invalid("DNS 配置必须是有效的 YAML 映射"))?;
        if !dns.is_mapping() || dns.get("dns").is_some() {
            return Err(invalid("请填写 dns 下的配置字段，不要包含外层 dns 键"));
        }
        let controller = &self.external_controller;
        let address = controller
            .address
            .parse::<std::net::SocketAddr>()
            .map_err(|_| invalid("控制器地址必须是 IP:端口，例如 127.0.0.1:9097"))?;
        if address.port() == 0 || (controller.enabled && address.port() == self.mixed_port) {
            return Err(invalid("控制器端口必须有效，且不能与代理端口相同"));
        }
        if controller.secret.len() > 1024
            || controller
                .secret
                .chars()
                .any(|c| c.is_control() || !c.is_ascii())
        {
            return Err(invalid(
                "API 密钥只能包含可打印的 ASCII 字符，最多 1024 字节",
            ));
        }
        if controller.enabled && controller.secret.trim().is_empty() {
            return Err(invalid("启用外部控制器前请设置 API 密钥"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_network_overrides() {
        let baseline = CoreNetworkSettings::default();
        assert!(baseline.validate().is_ok());
        let invalid = [
            CoreNetworkSettings {
                mixed_port: 0,
                ..baseline.clone()
            },
            CoreNetworkSettings {
                log_level: "trace".into(),
                ..baseline.clone()
            },
            CoreNetworkSettings {
                dns_yaml: "[broken".into(),
                ..baseline.clone()
            },
            CoreNetworkSettings {
                dns_yaml: "dns: {}".into(),
                ..baseline.clone()
            },
            CoreNetworkSettings {
                external_controller: ExternalControllerSettings {
                    enabled: true,
                    ..Default::default()
                },
                ..baseline.clone()
            },
            CoreNetworkSettings {
                external_controller: ExternalControllerSettings {
                    address: "127.0.0.1:0".into(),
                    ..Default::default()
                },
                ..baseline
            },
        ];
        for settings in invalid {
            assert!(settings.validate().is_err());
        }
    }
}
