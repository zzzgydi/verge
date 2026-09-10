use super::{AppError, ErrorCode, ProxyEndpoint, SystemProxyState};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

pub const DEFAULT_PROXY_BYPASS: &[&str] = &[
    "localhost",
    "127.0.0.1",
    "::1",
    "192.168.0.0/16",
    "10.0.0.0/8",
    "172.16.0.0/12",
    "*.local",
    "<local>",
];
pub const DEFAULT_PAC_SCRIPT: &str = "function FindProxyForURL(url, host) {\n  return \"PROXY %proxy_host%:%mixed-port%; SOCKS5 %proxy_host%:%mixed-port%; DIRECT\";\n}\n";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemProxySettings {
    pub host: String,
    pub pac_mode: bool,
    pub guard_enabled: bool,
    pub guard_interval_secs: u64,
    pub use_default_bypass: bool,
    pub validate_bypass: bool,
    pub bypass: Vec<String>,
    pub pac_script: String,
}
impl Default for SystemProxySettings {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            pac_mode: false,
            guard_enabled: false,
            guard_interval_secs: 30,
            use_default_bypass: true,
            validate_bypass: true,
            bypass: Vec::new(),
            pac_script: DEFAULT_PAC_SCRIPT.into(),
        }
    }
}
impl SystemProxySettings {
    pub fn validate(&self) -> Result<(), AppError> {
        let invalid = |message| AppError::new(ErrorCode::InvalidInput, message);
        if !valid_host(&self.host) {
            return Err(invalid("proxy host must be an IP address or hostname"));
        }
        if !(1..=86400).contains(&self.guard_interval_secs) {
            return Err(invalid(
                "proxy guard interval must be between 1 and 86400 seconds",
            ));
        }
        if self.bypass.len() > 512 || self.bypass.iter().map(String::len).sum::<usize>() > 8192 {
            return Err(invalid("proxy bypass list is too large"));
        }
        for domain in &self.bypass {
            // Disabling format checks must not permit command separators/control bytes.
            if domain.is_empty()
                || domain
                    .chars()
                    .any(|c| c.is_control() || c.is_whitespace() || matches!(c, ',' | ';'))
                || domain.starts_with('-')
                || domain.eq_ignore_ascii_case("Empty")
            {
                return Err(invalid(
                    "proxy bypass entries must be nonempty and contain no whitespace or separators",
                ));
            }
            if self.validate_bypass && !valid_bypass(domain) {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    format!("invalid proxy bypass entry: {domain}"),
                ));
            }
        }
        if self.pac_script.len() > 64 * 1024
            || self.pac_script.contains('\0')
            || self.pac_script.trim().is_empty()
        {
            return Err(invalid("PAC script must contain 1–65536 bytes without NUL"));
        }
        Ok(())
    }
    pub fn endpoint(&self, port: u16) -> Result<ProxyEndpoint, AppError> {
        self.validate()?;
        ProxyEndpoint::new(&self.host, port)
    }
    pub fn effective_bypass(&self) -> Vec<String> {
        let mut values = Vec::new();
        // Match Rev's backend: defaults are retained when custom list is empty,
        // or prepended to the user's custom entries when requested.
        if self.use_default_bypass || self.bypass.is_empty() {
            values.extend(DEFAULT_PROXY_BYPASS.iter().map(|v| (*v).to_owned()));
        }
        for value in &self.bypass {
            if !values.contains(value) {
                values.push(value.clone());
            }
        }
        values
    }
    pub fn render_pac(&self, port: u16) -> String {
        let host = if self.host.parse::<IpAddr>().is_ok_and(|ip| ip.is_ipv6()) {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        self.pac_script
            .replace("%proxy_host%", &host)
            .replace("%mixed-port%", &port.to_string())
    }
}
fn valid_host(host: &str) -> bool {
    host.parse::<IpAddr>().is_ok()
        || (host.len() <= 253
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            }))
}
fn valid_bypass(value: &str) -> bool {
    if value == "<local>" || value == "*" || valid_host(value) {
        return true;
    }
    if let Some((ip, bits)) = value.split_once('/') {
        return ip
            .parse::<IpAddr>()
            .ok()
            .zip(bits.parse::<u8>().ok())
            .is_some_and(|(ip, bits)| bits <= if ip.is_ipv4() { 32 } else { 128 });
    }
    value.contains('*') && valid_host(&value.replace('*', "x"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SystemProxyTarget {
    Manual {
        endpoint: ProxyEndpoint,
        bypass: Vec<String>,
    },
    Pac {
        url: String,
    },
}
impl SystemProxyTarget {
    pub fn matches(&self, state: &SystemProxyState) -> bool {
        !state.services.is_empty()
            && state.services.iter().all(|service| match self {
                Self::Manual { endpoint, bypass } => {
                    [&service.web, &service.secure_web, &service.socks]
                        .iter()
                        .all(|p| p.enabled && &p.endpoint == endpoint)
                        && !service.auto_proxy.enabled
                        && &service.bypass == bypass
                }
                Self::Pac { url } => {
                    service.auto_proxy.enabled
                        && service.auto_proxy.url.as_ref() == Some(url)
                        && !service.web.enabled
                        && !service.secure_web.enabled
                        && !service.socks.enabled
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_settings_and_default_bypass_are_safe() {
        let settings: SystemProxySettings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings, SystemProxySettings::default());
        let custom = SystemProxySettings {
            bypass: vec!["example.com".into(), "localhost".into()],
            ..settings
        };
        assert_eq!(
            custom
                .effective_bypass()
                .iter()
                .filter(|s| *s == "localhost")
                .count(),
            1
        );
        assert!(custom.effective_bypass().contains(&"example.com".into()));
    }
    #[test]
    fn bypass_validation_supports_ip_cidr_wildcards_and_can_be_relaxed() {
        let mut settings = SystemProxySettings {
            bypass: vec![
                "::1/128".into(),
                "10.0.0.0/8".into(),
                "*.example.com".into(),
                "<local>".into(),
            ],
            ..Default::default()
        };
        settings.validate().unwrap();
        for invalid in [
            "10.0.0.0/33",
            "https://example.com",
            "--option",
            "Empty",
            "a\nb",
        ] {
            settings.bypass = vec![invalid.into()];
            assert!(settings.validate().is_err());
        }
        settings.validate_bypass = false;
        settings.bypass = vec!["unusual_pattern".into()];
        settings.validate().unwrap();
        settings.bypass = vec!["a\0b".into()];
        assert!(settings.validate().is_err());
        settings.host = "https://localhost".into();
        assert!(settings.validate().is_err());
    }
    #[test]
    fn unified_indicator_rejects_partial_or_mixed_services() {
        use crate::domain::{AutoProxyState, ProxyProtocolState, SystemProxyServiceState};
        let protocol = ProxyProtocolState {
            enabled: true,
            endpoint: ProxyEndpoint::new("127.0.0.1", 7897).unwrap(),
        };
        let manual = SystemProxyServiceState {
            service: "Wi-Fi".into(),
            web: protocol.clone(),
            secure_web: protocol.clone(),
            socks: protocol,
            auto_proxy: AutoProxyState {
                enabled: false,
                url: None,
            },
            bypass: vec![],
        };
        let mut state = SystemProxyState {
            services: vec![manual.clone(), manual],
            recovery_pending: false,
        };
        assert!(state.unified_enabled());
        state.services[1].socks.enabled = false;
        assert!(!state.unified_enabled());
        for service in &mut state.services {
            service.web.enabled = false;
            service.secure_web.enabled = false;
            service.socks.enabled = false;
            service.auto_proxy.enabled = true;
        }
        assert!(!state.unified_enabled(), "PAC needs a URL");
        for service in &mut state.services {
            service.auto_proxy.url = Some("http://127.0.0.1/proxy.pac".into());
        }
        assert!(state.unified_enabled());
        state.services[1].auto_proxy.url = Some("http://127.0.0.1/another.pac".into());
        assert!(!state.unified_enabled());
    }

    #[test]
    fn pac_uses_current_port_without_mutating_template() {
        let settings = SystemProxySettings {
            host: "::1".into(),
            ..Default::default()
        };
        assert!(settings.render_pac(7897).contains("[::1]:7897"));
        assert!(settings.render_pac(9000).contains("[::1]:9000"));
        assert!(settings.pac_script.contains("%mixed-port%"));
    }
}
