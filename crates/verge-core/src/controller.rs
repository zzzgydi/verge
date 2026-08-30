use std::{
    fmt,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use verge_domain::{
    AppError, ErrorCode, NetworkSettings, ProviderKind, ProviderSummary, ProxyGroup, RuleEntry,
    RunMode,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControllerRequest {
    pub method: &'static str,
    pub path: String,
    pub body: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControllerResponse {
    pub status: u16,
    pub body: String,
}

pub trait ControllerTransport {
    fn send(&mut self, request: ControllerRequest) -> Result<ControllerResponse, AppError>;
}

pub struct TcpControllerTransport {
    controller: SocketAddr,
    secret: String,
    timeout: Duration,
}

impl TcpControllerTransport {
    pub fn new(
        controller: SocketAddr,
        secret: impl Into<String>,
        timeout: Duration,
    ) -> Result<Self, AppError> {
        if !controller.ip().is_loopback() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Mihomo controller must use a loopback address",
            ));
        }
        let secret = secret.into();
        if secret.chars().any(char::is_control) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Mihomo controller secret contains control characters",
            ));
        }
        Ok(Self {
            controller,
            secret,
            timeout,
        })
    }
}

impl ControllerTransport for TcpControllerTransport {
    fn send(&mut self, request: ControllerRequest) -> Result<ControllerResponse, AppError> {
        if !request.path.starts_with('/') || request.path.contains(['\r', '\n']) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "invalid Mihomo controller request path",
            ));
        }
        let mut stream = TcpStream::connect_timeout(&self.controller, self.timeout)
            .map_err(controller_io_error)?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(controller_io_error)?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(controller_io_error)?;
        let body = request.body.unwrap_or_default();
        write!(
            stream,
            "{} {} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            request.method,
            request.path,
            self.controller,
            self.secret,
            body.len(),
            body
        )
        .map_err(controller_io_error)?;
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .map_err(controller_io_error)?;
        parse_http_response(&response)
    }
}

pub struct MihomoClient<T> {
    transport: T,
}

impl<T: ControllerTransport> MihomoClient<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    pub fn version(&mut self) -> Result<String, AppError> {
        #[derive(Deserialize)]
        struct VersionResponse {
            version: String,
        }
        let response: VersionResponse = self.json("GET", "/version", None)?;
        Ok(response.version)
    }

    pub fn mode(&mut self) -> Result<RunMode, AppError> {
        #[derive(Deserialize)]
        struct ConfigResponse {
            mode: RunMode,
        }
        let response: ConfigResponse = self.json("GET", "/configs", None)?;
        Ok(response.mode)
    }

    pub fn set_mode(&mut self, mode: RunMode) -> Result<(), AppError> {
        let body = serde_json::to_string(&serde_json::json!({ "mode": mode }))
            .map_err(controller_data_error)?;
        self.empty("PATCH", "/configs", Some(body))
    }

    pub fn network_settings(&mut self) -> Result<NetworkSettings, AppError> {
        let response: serde_json::Value = self.json("GET", "/configs", None)?;
        Ok(NetworkSettings {
            tun_enabled: response
                .pointer("/tun/enable")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            dns_enabled: response
                .pointer("/dns/enable")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            ipv6_enabled: response
                .get("ipv6")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        })
    }

    pub fn set_network_settings(&mut self, settings: &NetworkSettings) -> Result<(), AppError> {
        let body = serde_json::to_string(&serde_json::json!({
            "tun": { "enable": settings.tun_enabled },
            "dns": { "enable": settings.dns_enabled },
            "ipv6": settings.ipv6_enabled,
        }))
        .map_err(controller_data_error)?;
        self.empty("PATCH", "/configs", Some(body))
    }

    pub fn proxy_groups(&mut self) -> Result<Vec<ProxyGroup>, AppError> {
        let response: serde_json::Value = self.json("GET", "/proxies", None)?;
        let proxies = response
            .get("proxies")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| controller_error("Mihomo proxies response has no proxies object"))?;
        let mut groups = Vec::new();
        for (name, proxy) in proxies {
            let members = proxy
                .get("all")
                .and_then(serde_json::Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if members.is_empty() {
                continue;
            }
            groups.push(ProxyGroup {
                name: name.clone(),
                kind: proxy
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("Unknown")
                    .to_owned(),
                selected: proxy
                    .get("now")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                members,
            });
        }
        groups.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(groups)
    }

    pub fn rules(&mut self) -> Result<Vec<RuleEntry>, AppError> {
        #[derive(Deserialize)]
        struct RulesResponse {
            rules: Vec<RawRule>,
        }
        #[derive(Deserialize)]
        struct RawRule {
            #[serde(rename = "type")]
            kind: String,
            #[serde(default)]
            payload: String,
            proxy: String,
            #[serde(default)]
            size: i64,
        }
        let response: RulesResponse = self.json("GET", "/rules", None)?;
        Ok(response
            .rules
            .into_iter()
            .map(|rule| RuleEntry {
                kind: rule.kind,
                payload: rule.payload,
                proxy: rule.proxy,
                size: rule.size,
            })
            .collect())
    }

    pub fn providers(&mut self) -> Result<Vec<ProviderSummary>, AppError> {
        let mut providers = self.providers_at("/providers/proxies", ProviderKind::Proxy)?;
        providers.extend(self.providers_at("/providers/rules", ProviderKind::Rule)?);
        providers.sort_by(|left, right| {
            format!("{:?}:{}", left.kind, left.name)
                .cmp(&format!("{:?}:{}", right.kind, right.name))
        });
        Ok(providers)
    }

    pub fn update_provider(&mut self, kind: ProviderKind, name: &str) -> Result<(), AppError> {
        if name.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "provider name is empty",
            ));
        }
        let category = match kind {
            ProviderKind::Proxy => "proxies",
            ProviderKind::Rule => "rules",
        };
        self.empty(
            "PUT",
            &format!("/providers/{category}/{}", encode_path_segment(name)),
            None,
        )
    }

    fn providers_at(
        &mut self,
        path: &str,
        kind: ProviderKind,
    ) -> Result<Vec<ProviderSummary>, AppError> {
        let response: serde_json::Value = self.json("GET", path, None)?;
        let providers = response
            .get("providers")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| controller_error("Mihomo provider response has no providers object"))?;
        Ok(providers
            .iter()
            .map(|(name, provider)| ProviderSummary {
                name: name.clone(),
                kind,
                vehicle: provider
                    .get("vehicleType")
                    .or_else(|| provider.get("type"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("Unknown")
                    .to_owned(),
                updated_at: provider
                    .get("updatedAt")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                item_count: provider
                    .get("proxies")
                    .and_then(serde_json::Value::as_array)
                    .map(Vec::len)
                    .or_else(|| {
                        provider
                            .get("ruleCount")
                            .and_then(serde_json::Value::as_u64)
                            .and_then(|count| usize::try_from(count).ok())
                    })
                    .unwrap_or(0),
            })
            .collect())
    }

    pub fn select_proxy(&mut self, group: &str, proxy: &str) -> Result<(), AppError> {
        let group = encode_path_segment(group);
        let body = serde_json::to_string(&serde_json::json!({ "name": proxy }))
            .map_err(controller_data_error)?;
        self.empty("PUT", &format!("/proxies/{group}"), Some(body))
    }

    pub fn delay(&mut self, proxy: &str, test_url: &str, timeout_ms: u32) -> Result<u32, AppError> {
        #[derive(Deserialize)]
        struct DelayResponse {
            delay: u32,
        }
        if !(test_url.starts_with("http://") || test_url.starts_with("https://")) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "delay test URL must use HTTP or HTTPS",
            ));
        }
        let proxy = encode_path_segment(proxy);
        let url = utf8_percent_encode(test_url, NON_ALPHANUMERIC);
        let path = format!("/proxies/{proxy}/delay?timeout={timeout_ms}&url={url}");
        let response: DelayResponse = self.json("GET", &path, None)?;
        Ok(response.delay)
    }

    pub fn close_connection(&mut self, id: &str) -> Result<(), AppError> {
        if id.is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "connection id is empty",
            ));
        }
        self.empty(
            "DELETE",
            &format!("/connections/{}", encode_path_segment(id)),
            None,
        )
    }

    fn empty(
        &mut self,
        method: &'static str,
        path: &str,
        body: Option<String>,
    ) -> Result<(), AppError> {
        let response = self.transport.send(ControllerRequest {
            method,
            path: path.to_owned(),
            body,
        })?;
        ensure_success(&response).map(|_| ())
    }

    fn json<D: for<'de> Deserialize<'de>>(
        &mut self,
        method: &'static str,
        path: &str,
        body: Option<String>,
    ) -> Result<D, AppError> {
        let response = self.transport.send(ControllerRequest {
            method,
            path: path.to_owned(),
            body,
        })?;
        serde_json::from_str(ensure_success(&response)?).map_err(controller_data_error)
    }
}

fn parse_http_response(response: &[u8]) -> Result<ControllerResponse, AppError> {
    let separator = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| controller_error("invalid HTTP response from Mihomo"))?;
    let head = std::str::from_utf8(&response[..separator]).map_err(controller_data_error)?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| controller_error("invalid HTTP status from Mihomo"))?;
    let encoded_body = &response[separator + 4..];
    let body = if head.lines().skip(1).any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.eq_ignore_ascii_case("transfer-encoding")
                && value
                    .split(',')
                    .any(|encoding| encoding.trim().eq_ignore_ascii_case("chunked"))
        })
    }) {
        String::from_utf8(decode_chunked(encoded_body)?).map_err(controller_data_error)?
    } else {
        String::from_utf8(encoded_body.to_vec()).map_err(controller_data_error)?
    };
    Ok(ControllerResponse { status, body })
}

fn decode_chunked(mut encoded: &[u8]) -> Result<Vec<u8>, AppError> {
    let mut decoded = Vec::new();
    loop {
        let line_end = encoded
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| controller_error("invalid chunked response from Mihomo"))?;
        let size_text = std::str::from_utf8(&encoded[..line_end])
            .map_err(controller_data_error)?
            .split(';')
            .next()
            .unwrap_or_default()
            .trim();
        let size = usize::from_str_radix(size_text, 16).map_err(controller_data_error)?;
        encoded = &encoded[line_end + 2..];
        if size == 0 {
            return Ok(decoded);
        }
        if encoded.len() < size + 2 || &encoded[size..size + 2] != b"\r\n" {
            return Err(controller_error("truncated chunked response from Mihomo"));
        }
        decoded.extend_from_slice(&encoded[..size]);
        encoded = &encoded[size + 2..];
    }
}

fn ensure_success(response: &ControllerResponse) -> Result<&str, AppError> {
    if (200..300).contains(&response.status) {
        Ok(&response.body)
    } else {
        Err(controller_error(format!(
            "Mihomo controller returned HTTP {}: {}",
            response.status,
            response.body.trim()
        )))
    }
}

fn encode_path_segment(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

fn controller_io_error(error: std::io::Error) -> AppError {
    controller_error(error.to_string())
}

fn controller_data_error(error: impl fmt::Display) -> AppError {
    controller_error(format!("invalid Mihomo controller response: {error}"))
}

fn controller_error(message: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::CoreUnavailable, message)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    struct FakeTransport {
        responses: VecDeque<ControllerResponse>,
        requests: Vec<ControllerRequest>,
    }

    impl FakeTransport {
        fn new(responses: impl IntoIterator<Item = ControllerResponse>) -> Self {
            Self {
                responses: responses.into_iter().collect(),
                requests: Vec::new(),
            }
        }
    }

    impl ControllerTransport for FakeTransport {
        fn send(&mut self, request: ControllerRequest) -> Result<ControllerResponse, AppError> {
            self.requests.push(request);
            Ok(self.responses.pop_front().unwrap())
        }
    }

    fn response(status: u16, body: &str) -> ControllerResponse {
        ControllerResponse {
            status,
            body: body.into(),
        }
    }

    #[test]
    fn mode_contract_uses_configs_patch() {
        let transport = FakeTransport::new([response(204, "")]);
        let mut client = MihomoClient::new(transport);
        client.set_mode(RunMode::Global).unwrap();
        let request = &client.transport.requests[0];
        assert_eq!(request.method, "PATCH");
        assert_eq!(request.path, "/configs");
        assert_eq!(request.body.as_deref(), Some(r#"{"mode":"global"}"#));
    }

    #[test]
    fn network_settings_round_trip_through_configs_contract() {
        let transport = FakeTransport::new([
            response(
                200,
                r#"{"tun":{"enable":true},"dns":{"enable":true},"ipv6":false}"#,
            ),
            response(204, ""),
        ]);
        let mut client = MihomoClient::new(transport);
        assert_eq!(
            client.network_settings().unwrap(),
            NetworkSettings {
                tun_enabled: true,
                dns_enabled: true,
                ipv6_enabled: false,
            }
        );
        client
            .set_network_settings(&NetworkSettings {
                tun_enabled: false,
                dns_enabled: true,
                ipv6_enabled: true,
            })
            .unwrap();
        let request = &client.transport.requests[1];
        assert_eq!(request.method, "PATCH");
        assert_eq!(request.path, "/configs");
        let body: serde_json::Value =
            serde_json::from_str(request.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["tun"]["enable"], false);
        assert_eq!(body["dns"]["enable"], true);
        assert_eq!(body["ipv6"], true);
    }

    #[test]
    fn proxies_contract_extracts_groups() {
        let transport = FakeTransport::new([response(
            200,
            r#"{"proxies":{"Node":{"type":"Shadowsocks"},"Select":{"type":"Selector","now":"Node","all":["Node","DIRECT"]}}}"#,
        )]);
        let mut client = MihomoClient::new(transport);
        assert_eq!(
            client.proxy_groups().unwrap(),
            [ProxyGroup {
                name: "Select".into(),
                kind: "Selector".into(),
                selected: Some("Node".into()),
                members: vec!["Node".into(), "DIRECT".into()],
            }]
        );
    }

    #[test]
    fn selection_and_delay_encode_untrusted_url_parts() {
        let transport = FakeTransport::new([
            response(204, ""),
            response(200, r#"{"delay":42}"#),
            response(204, ""),
        ]);
        let mut client = MihomoClient::new(transport);
        client.select_proxy("Auto / Select", "Node A").unwrap();
        assert_eq!(
            client
                .delay("Node A", "https://example.com/generate_204", 5_000)
                .unwrap(),
            42
        );
        assert_eq!(
            client.transport.requests[0].path,
            "/proxies/Auto%20%2F%20Select"
        );
        assert!(
            client.transport.requests[1]
                .path
                .starts_with("/proxies/Node%20A/delay?")
        );
        assert!(
            client.transport.requests[1]
                .path
                .contains("https%3A%2F%2Fexample")
        );
        client.close_connection("id / 1").unwrap();
        assert_eq!(client.transport.requests[2].method, "DELETE");
        assert_eq!(
            client.transport.requests[2].path,
            "/connections/id%20%2F%201"
        );
        assert_eq!(
            client.close_connection("").unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn controller_errors_keep_status_and_body() {
        let transport = FakeTransport::new([response(401, r#"{"message":"Unauthorized"}"#)]);
        let mut client = MihomoClient::new(transport);
        let error = client.version().unwrap_err();
        assert!(error.message.contains("401"));
        assert!(error.message.contains("Unauthorized"));
    }

    #[test]
    fn rules_and_providers_use_mihomo_contracts() {
        let transport = FakeTransport::new([
            response(
                200,
                r#"{"rules":[{"type":"DomainSuffix","payload":"example.com","proxy":"Proxy","size":7}]}"#,
            ),
            response(
                200,
                r#"{"providers":{"nodes":{"vehicleType":"HTTP","updatedAt":"now","proxies":[{},{}]}}}"#,
            ),
            response(
                200,
                r#"{"providers":{"ads":{"vehicleType":"File","updatedAt":"then","ruleCount":9}}}"#,
            ),
            response(204, ""),
        ]);
        let mut client = MihomoClient::new(transport);
        assert_eq!(
            client.rules().unwrap(),
            [RuleEntry {
                kind: "DomainSuffix".into(),
                payload: "example.com".into(),
                proxy: "Proxy".into(),
                size: 7,
            }]
        );
        let providers = client.providers().unwrap();
        assert_eq!(providers.len(), 2);
        assert!(providers.iter().any(|provider| {
            provider.name == "nodes"
                && provider.kind == ProviderKind::Proxy
                && provider.item_count == 2
        }));
        assert!(providers.iter().any(|provider| {
            provider.name == "ads"
                && provider.kind == ProviderKind::Rule
                && provider.item_count == 9
        }));
        client
            .update_provider(ProviderKind::Rule, "ads / rules")
            .unwrap();
        assert_eq!(
            client.transport.requests[3].path,
            "/providers/rules/ads%20%2F%20rules"
        );
    }

    #[test]
    fn http_parser_decodes_chunked_json_body() {
        let response = parse_http_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n7\r\n{\"ok\":t\r\n4\r\nrue}\r\n0\r\n\r\n",
        )
        .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, r#"{"ok":true}"#);
    }

    #[test]
    fn http_parser_rejects_truncated_chunks() {
        let error = parse_http_response(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nabc\r\n",
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::CoreUnavailable);
    }
}
