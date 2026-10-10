use super::{ControllerEndpoint, endpoint::ControllerStream};
use std::{
    fmt,
    io::{Read, Write},
    net::SocketAddr,
    time::{Duration, Instant},
};

use crate::domain::{
    AppError, ErrorCode, NetworkSettings, ProviderKind, ProviderSummary, ProxyDetails, ProxyGroup,
    ProxySnapshot, RuleEntry, RunMode,
};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControllerRequest {
    pub method: &'static str,
    pub path: String,
    pub body: Option<String>,
    /// Long-running operations supply a total response budget. Connect/write stay bounded.
    pub response_timeout: Option<Duration>,
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
    controller: ControllerEndpoint,
    secret: String,
    timeout: Duration,
}

impl TcpControllerTransport {
    #[cfg(unix)]
    pub fn unix(path: std::path::PathBuf, timeout: Duration) -> Result<Self, AppError> {
        let endpoint = ControllerEndpoint::Unix(path);
        if !endpoint.is_local() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "controller socket must be absolute",
            ));
        }
        Ok(Self {
            controller: endpoint,
            secret: String::new(),
            timeout,
        })
    }

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
            controller: controller.into(),
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
        let mut stream = self
            .controller
            .connect(self.timeout)
            .map_err(controller_io_error)?;
        let body = request.body.unwrap_or_default();
        write!(
            stream,
            "{} {} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            request.method,
            request.path,
            self.controller.host(),
            self.secret,
            body.len(),
            body
        )
        .map_err(controller_io_error)?;
        read_response(
            &mut stream,
            request.response_timeout.unwrap_or(self.timeout),
        )
    }
}

fn read_response(
    stream: &mut ControllerStream,
    timeout: Duration,
) -> Result<ControllerResponse, AppError> {
    #[cfg(unix)]
    stream.set_nonblocking(true).map_err(controller_io_error)?;
    let mut response = Vec::new();
    let deadline = Instant::now() + timeout;
    let mut buffer = [0; 8192];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(controller_io_error(std::io::ErrorKind::TimedOut.into()));
        }
        match stream.wait_readable(remaining) {
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) =>
            {
                continue;
            }
            Err(error) => return Err(controller_io_error(error)),
            Ok(()) => {}
        }
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                response.extend_from_slice(&buffer[..count]);
                if response.len() > 16 * 1024 * 1024 {
                    return Err(controller_error("Mihomo response exceeds 16 MiB"));
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) =>
            {
                continue;
            }
            Err(error) => return Err(controller_io_error(error)),
        }
    }
    parse_http_response(&response)
}

fn proxy_details(proxy: &serde_json::Value) -> ProxyDetails {
    let flag = |key: &str| {
        proxy
            .get(key)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    };
    ProxyDetails {
        kind: proxy
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Unknown")
            .to_owned(),
        udp: proxy.get("udp").and_then(serde_json::Value::as_bool),
        xudp: flag("xudp"),
        tfo: flag("tfo"),
        mptcp: flag("mptcp"),
        smux: flag("smux"),
        hidden: flag("hidden"),
        selected: proxy
            .get("now")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        delay: proxy
            .get("history")
            .and_then(serde_json::Value::as_array)
            .and_then(|history| history.last())
            .and_then(|entry| entry.get("delay"))
            .and_then(serde_json::Value::as_u64)
            .and_then(|ms| u32::try_from(ms).ok()),
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

    /// Read the same metadata as the connection WebSocket, on demand.
    pub fn connections(&mut self) -> Result<crate::domain::ConnectionSnapshot, AppError> {
        let response: serde_json::Value = self.json("GET", "/connections", None)?;
        super::realtime::parse_connections(&response.to_string())
    }

    /// Query the running core's resolver, not the OS resolver or an arbitrary URL.
    pub fn query_dns(&mut self, host: &str, kind: &str) -> Result<serde_json::Value, AppError> {
        if !["A", "AAAA", "CNAME"].contains(&kind) {
            return Err(controller_error("Unsupported DNS query type"));
        }
        let response = self.transport.send(ControllerRequest {
            method: "GET",
            path: format!("/dns/query?name={}&type={kind}", encode_path_segment(host)),
            body: None,
            // Mihomo v1.19.26 allows five seconds for DNS; leave time for its response.
            response_timeout: Some(Duration::from_secs(7)),
        })?;
        serde_json::from_str(ensure_success(&response)?).map_err(controller_data_error)
    }

    pub fn mode(&mut self) -> Result<RunMode, AppError> {
        #[derive(Deserialize)]
        struct ConfigResponse {
            mode: RunMode,
        }
        let response: ConfigResponse = self.json("GET", "/configs", None)?;
        Ok(response.mode)
    }

    pub fn log_level(&mut self) -> Result<String, AppError> {
        #[derive(Deserialize)]
        struct ConfigResponse {
            #[serde(rename = "log-level")]
            log_level: String,
        }
        let response: ConfigResponse = self.json("GET", "/configs", None)?;
        if ["debug", "info", "warning", "error", "silent"].contains(&response.log_level.as_str()) {
            Ok(response.log_level)
        } else {
            Err(controller_error("Mihomo returned an unknown log level"))
        }
    }

    pub fn set_mode(&mut self, mode: RunMode) -> Result<(), AppError> {
        let body = serde_json::to_string(&serde_json::json!({ "mode": mode }))
            .map_err(controller_data_error)?;
        self.empty("PATCH", "/configs", Some(body))
    }

    pub(crate) fn verify_tun(&mut self, device: &str) -> Result<(), AppError> {
        let response: serde_json::Value = self.json("GET", "/configs", None)?;
        let tun = &response["tun"];
        if tun["enable"] == true && tun["device"] == device && tun["file-descriptor"] == 3 {
            Ok(())
        } else {
            Err(controller_error(
                "Mihomo did not start the leased TUN listener",
            ))
        }
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

    pub fn proxy_groups(&mut self) -> Result<ProxySnapshot, AppError> {
        let response: serde_json::Value = self.json("GET", "/proxies", None)?;
        let proxies = response
            .get("proxies")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| controller_error("Mihomo proxies response has no proxies object"))?;
        let mut groups = Vec::new();
        let mut details = std::collections::BTreeMap::new();
        for (name, proxy) in proxies {
            details.insert(name.clone(), proxy_details(proxy));
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
            if !proxy.get("all").is_some_and(serde_json::Value::is_array) {
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
        // Provider-only nodes may be omitted from /proxies. Resolve unique names;
        // ambiguous or unavailable details remain unknown instead of guessing capabilities.
        let missing: std::collections::HashSet<_> = groups
            .iter()
            .flat_map(|g| &g.members)
            .filter(|name| !details.contains_key(*name))
            .cloned()
            .collect();
        if !missing.is_empty()
            && let Ok(response) = self.json::<serde_json::Value>("GET", "/providers/proxies", None)
        {
            let mut candidates = std::collections::HashMap::new();
            if let Some(providers) = response
                .get("providers")
                .and_then(serde_json::Value::as_object)
            {
                for provider in providers.values() {
                    for proxy in provider
                        .get("proxies")
                        .and_then(serde_json::Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        if let Some(name) = proxy
                            .get("name")
                            .and_then(serde_json::Value::as_str)
                            .filter(|name| missing.contains(*name))
                        {
                            candidates
                                .entry(name.to_owned())
                                .and_modify(|value| *value = None)
                                .or_insert_with(|| Some(proxy_details(proxy)));
                        }
                    }
                }
            }
            details.extend(
                candidates
                    .into_iter()
                    .filter_map(|(name, details)| details.map(|details| (name, details))),
            );
        }
        groups.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(ProxySnapshot {
            groups,
            proxies: details,
        })
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
        if !(1..=60_000).contains(&timeout_ms) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "delay test timeout must be between 1 and 60000 ms",
            ));
        }
        let proxy = encode_path_segment(proxy);
        let url = utf8_percent_encode(test_url, NON_ALPHANUMERIC);
        let path = format!("/proxies/{proxy}/delay?timeout={timeout_ms}&url={url}");
        let response = self.transport.send(ControllerRequest {
            method: "GET",
            path,
            body: None,
            // Allow Mihomo to finish its node test and return the result (including timeout).
            response_timeout: Some(
                Duration::from_millis(u64::from(timeout_ms)) + Duration::from_secs(2),
            ),
        })?;
        if !(200..300).contains(&response.status) {
            return Err(AppError::new(
                if response.status == 504 {
                    ErrorCode::RequestTimeout
                } else {
                    ErrorCode::ProxyDelayFailed
                },
                format!(
                    "Proxy delay test returned HTTP {}: {}",
                    response.status,
                    response.body.trim()
                ),
            ));
        }
        let response: DelayResponse = serde_json::from_str(&response.body).map_err(|error| {
            AppError::new(
                ErrorCode::ProxyDelayFailed,
                format!("Invalid proxy delay response: {error}"),
            )
        })?;
        Ok(response.delay)
    }

    pub fn close_all_connections(&mut self) -> Result<(), AppError> {
        self.empty("DELETE", "/connections", None)
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
            response_timeout: None,
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
            response_timeout: None,
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
    if matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) {
        AppError::new(
            ErrorCode::RequestTimeout,
            "Timed out waiting for Mihomo controller response",
        )
    } else {
        controller_error(error.to_string())
    }
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
    fn effective_log_level_accepts_only_mihomo_levels() {
        for level in ["debug", "info", "warning", "error", "silent"] {
            let mut client = MihomoClient::new(FakeTransport::new([response(
                200,
                &format!(r#"{{"log-level":"{level}"}}"#),
            )]));
            assert_eq!(client.log_level().unwrap(), level);
            assert_eq!(client.transport.requests[0].path, "/configs");
        }
        let mut client = MihomoClient::new(FakeTransport::new([response(
            200,
            r#"{"log-level":"trace"}"#,
        )]));
        assert!(client.log_level().is_err());
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
            r#"{"proxies":{"Node":{"type":"Shadowsocks"},"DIRECT":{"type":"Direct"},"Select":{"type":"Selector","now":"Node","all":["Node","DIRECT"]}}}"#,
        )]);
        let mut client = MihomoClient::new(transport);
        assert_eq!(
            client.proxy_groups().unwrap().groups,
            [ProxyGroup {
                name: "Select".into(),
                kind: "Selector".into(),
                selected: Some("Node".into()),
                members: vec!["Node".into(), "DIRECT".into()],
            }]
        );
    }

    #[test]
    fn proxy_snapshot_keeps_protocol_capabilities_provider_nodes_and_empty_groups() {
        let transport = FakeTransport::new([
            response(
                200,
                r#"{"proxies":{"Route":{"type":"Selector","now":"Remote","all":["Remote","Ambiguous","Missing"]},"Empty":{"type":"Selector","all":[]},"Core":{"type":"VLESS","udp":false,"tfo":true,"history":[{"delay":8},{"delay":0}]}}}"#,
            ),
            response(
                200,
                r#"{"providers":{"one":{"proxies":[{"name":"Remote","type":"Hysteria2","udp":true,"xudp":true,"smux":true},{"name":"Ambiguous","type":"Trojan"}]},"two":{"proxies":[{"name":"Ambiguous","type":"Shadowsocks"}]}}}"#,
            ),
        ]);
        let snapshot = MihomoClient::new(transport).proxy_groups().unwrap();
        assert_eq!(snapshot.groups.len(), 2);
        assert!(
            snapshot
                .groups
                .iter()
                .any(|g| g.name == "Empty" && g.members.is_empty())
        );
        assert_eq!(snapshot.proxies["Core"].udp, Some(false));
        assert_eq!(snapshot.proxies["Core"].delay, Some(0));
        assert!(snapshot.proxies["Core"].tfo);
        assert_eq!(snapshot.proxies["Remote"].kind, "Hysteria2");
        assert_eq!(snapshot.proxies["Remote"].udp, Some(true));
        assert!(snapshot.proxies["Remote"].xudp && snapshot.proxies["Remote"].smux);
        assert!(!snapshot.proxies.contains_key("Ambiguous"));
        assert!(!snapshot.proxies.contains_key("Missing"));
    }

    #[test]
    fn close_all_connections_uses_collection_delete_and_reports_failure() {
        let mut client = MihomoClient::new(FakeTransport::new([
            response(204, ""),
            response(500, "failed"),
        ]));
        client.close_all_connections().unwrap();
        assert_eq!(client.transport.requests[0].method, "DELETE");
        assert_eq!(client.transport.requests[0].path, "/connections");
        assert!(client.close_all_connections().is_err());
    }

    #[test]
    fn dns_queries_have_a_separate_response_budget() {
        let mut client = MihomoClient::new(FakeTransport::new([
            response(200, r#"{"Status":0,"Answer":[{"data":"203.0.113.42"}]}"#),
            response(200, r#"{"Status":3}"#),
            response(504, "DNS timed out"),
            response(200, r#"{"version":"test"}"#),
        ]));
        let answer = client.query_dns("example.test", "A").unwrap();
        assert_eq!(answer["Answer"][0]["data"], "203.0.113.42");
        assert_eq!(
            client.query_dns("missing.test", "AAAA").unwrap()["Status"],
            3
        );
        assert!(client.query_dns("slow.test", "CNAME").is_err());
        assert!(client.query_dns("example.test", "TXT").is_err());
        assert_eq!(client.version().unwrap(), "test");
        let requests = &client.transport.requests;
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[0].path, "/dns/query?name=example%2Etest&type=A");
        assert!(requests[..3].iter().all(|request| {
            request.method == "GET"
                && request.body.is_none()
                && request.response_timeout == Some(Duration::from_secs(7))
        }));
        assert_eq!(requests[3].response_timeout, None);
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
        assert_eq!(client.transport.requests[0].response_timeout, None);
        assert_eq!(
            client.transport.requests[1].response_timeout,
            Some(Duration::from_secs(7))
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
        assert_eq!(client.transport.requests[2].response_timeout, None);
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
    fn delay_rejections_are_not_core_unavailable() {
        for (status, body, code) in [
            (504, r#"{"message":"Timeout"}"#, ErrorCode::RequestTimeout),
            (
                503,
                r#"{"message":"An error occurred in the delay test"}"#,
                ErrorCode::ProxyDelayFailed,
            ),
            (
                404,
                r#"{"message":"Proxy not found"}"#,
                ErrorCode::ProxyDelayFailed,
            ),
            (200, r#"{"delay":"invalid"}"#, ErrorCode::ProxyDelayFailed),
        ] {
            let mut client = MihomoClient::new(FakeTransport::new([response(status, body)]));
            assert_eq!(
                client
                    .delay("node", "https://example.invalid", 5000)
                    .unwrap_err()
                    .code,
                code
            );
        }
        let mut client = MihomoClient::new(FakeTransport::new([]));
        for timeout in [0, 60_001, u32::MAX] {
            assert_eq!(
                client
                    .delay("node", "https://example.invalid", timeout)
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidInput
            );
        }
        assert!(client.transport.requests.is_empty());
    }

    #[test]
    fn socket_deadlines_are_distinct_from_connection_failure() {
        for kind in [std::io::ErrorKind::WouldBlock, std::io::ErrorKind::TimedOut] {
            assert_eq!(
                controller_io_error(std::io::Error::from(kind)).code,
                ErrorCode::RequestTimeout
            );
        }
        assert_eq!(
            controller_io_error(std::io::Error::from(std::io::ErrorKind::ConnectionRefused)).code,
            ErrorCode::CoreUnavailable
        );
    }

    #[test]
    fn tcp_transport_reports_expired_response_deadline() {
        use std::{
            io::{BufRead, BufReader},
            net::TcpListener,
            thread,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(stream);
            loop {
                let mut line = String::new();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(300));
        });
        let transport =
            TcpControllerTransport::new(address, "", Duration::from_millis(50)).unwrap();
        let result = MihomoClient::new(transport).version();
        server.join().unwrap();
        assert_eq!(result.unwrap_err().code, ErrorCode::RequestTimeout);
    }

    #[cfg(unix)]
    #[test]
    fn closed_unix_peer_still_delivers_buffered_response() {
        use std::os::unix::net::UnixStream;

        let (client, mut server) = UnixStream::pair().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client
            .set_write_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        server
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 18\r\nConnection: close\r\n\r\n{\"version\":\"test\"}",
            )
            .unwrap();
        drop(server);

        let response =
            read_response(&mut ControllerStream::Unix(client), Duration::from_secs(2)).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, r#"{"version":"test"}"#);
    }

    #[cfg(unix)]
    #[test]
    fn unix_response_budget_expires_while_peer_drips_bytes() {
        use std::{os::unix::net::UnixStream, thread};

        let (client, mut server) = UnixStream::pair().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let sender = thread::spawn(move || {
            for _ in 0..4 {
                if server.write_all(b"a").is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(80));
            }
        });
        let error = read_response(
            &mut ControllerStream::Unix(client),
            Duration::from_millis(150),
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::RequestTimeout);
        sender.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "requires MIHOMO_SOCKET pointing to a running Mihomo controller"]
    fn running_mihomo_unix_responses_survive_peer_close() {
        let path = std::env::var_os("MIHOMO_SOCKET").expect("MIHOMO_SOCKET is required");
        let transport = TcpControllerTransport::unix(path.into(), Duration::from_secs(2)).unwrap();
        let mut client = MihomoClient::new(transport);
        for _ in 0..20 {
            assert!(!client.version().unwrap().is_empty());
            client.proxy_groups().unwrap();
            client.rules().unwrap();
        }
    }

    #[test]
    fn response_budget_expires_even_when_server_keeps_sending() {
        use std::{net::TcpListener, thread};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                }
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                    .unwrap();
                for byte in b"{\"version\":\"test\"}" {
                    thread::sleep(Duration::from_millis(35));
                    if stream.write_all(&[*byte]).is_err() {
                        break;
                    }
                }
            }
        });
        let mut transport =
            TcpControllerTransport::new(address, "", Duration::from_millis(150)).unwrap();
        for response_timeout in [None, Some(Duration::from_millis(150))] {
            let result = transport.send(ControllerRequest {
                method: "GET",
                path: "/version".into(),
                body: None,
                response_timeout,
            });
            assert_eq!(result.unwrap_err().code, ErrorCode::RequestTimeout);
        }
        server.join().unwrap();
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
