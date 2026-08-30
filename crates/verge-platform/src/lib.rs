use std::{
    fs,
    fs::File,
    io,
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::Command,
};

use fs2::FileExt;

use verge_domain::{
    AppError, AutoProxyState, ErrorCode, HelperStatus, ProxyEndpoint, ProxyProtocolState,
    SystemProxyServiceState, SystemProxyState,
};
use verge_helper_protocol::{HelperRequest, HelperResponse, MAX_REQUEST_BYTES, PROTOCOL_VERSION};

#[cfg(target_os = "macos")]
mod tray;
#[cfg(target_os = "macos")]
pub use tray::{TrayCommand, TrayService, TraySnapshot};

mod daemon;
pub use daemon::{daemon_socket_path, run_accessory_appkit_loop, spawn_daemon};

#[cfg(target_os = "macos")]
mod login_item;
#[cfg(target_os = "macos")]
pub use login_item::SmAppLoginItem;

/// 登录启动（launch at login）能力抽象，实现可注入、可 fake 测试。
pub trait LoginItemService {
    /// 当前是否已注册为登录项（待用户在系统设置中批准也视为已注册）。
    fn status(&mut self) -> Result<bool, AppError>;
    /// 注册或注销登录项。
    fn set_enabled(&mut self, enabled: bool) -> Result<(), AppError>;
}

/// 当前平台的默认登录启动实现。
pub fn default_login_item_service() -> Box<dyn LoginItemService + Send> {
    #[cfg(target_os = "macos")]
    {
        Box::new(SmAppLoginItem::new())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(UnsupportedLoginItem)
    }
}

#[cfg(not(target_os = "macos"))]
struct UnsupportedLoginItem;

#[cfg(not(target_os = "macos"))]
impl LoginItemService for UnsupportedLoginItem {
    fn status(&mut self) -> Result<bool, AppError> {
        Err(platform_error("launch at login is not supported on this platform"))
    }

    fn set_enabled(&mut self, _enabled: bool) -> Result<(), AppError> {
        Err(platform_error("launch at login is not supported on this platform"))
    }
}

const NETWORK_SETUP: &str = "/usr/sbin/networksetup";
const SECURITY: &str = "/usr/bin/security";
const OSASCRIPT: &str = "/usr/bin/osascript";

pub struct MacHelperClient {
    socket: PathBuf,
}

impl MacHelperClient {
    pub fn new(socket: impl Into<PathBuf>) -> Self {
        Self {
            socket: socket.into(),
        }
    }

    pub fn status(&self) -> HelperStatus {
        match self.exchange(&HelperRequest::Ping {
            protocol_version: PROTOCOL_VERSION,
        }) {
            Ok(HelperResponse::Pong { protocol_version }) => {
                HelperStatus::Ready { protocol_version }
            }
            Ok(HelperResponse::Error { message, .. }) => HelperStatus::Incompatible { message },
            Ok(response) => HelperStatus::Incompatible {
                message: format!("unexpected helper response: {response:?}"),
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => HelperStatus::NotInstalled,
            Err(error) => HelperStatus::Incompatible {
                message: error.to_string(),
            },
        }
    }

    fn exchange(&self, request: &HelperRequest) -> io::Result<HelperResponse> {
        let mut stream = UnixStream::connect(&self.socket)?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(std::time::Duration::from_secs(2)))?;
        serde_json::to_writer(&mut stream, request)?;
        stream.write_all(b"\n")?;
        let mut response = String::new();
        BufReader::new(stream)
            .take(MAX_REQUEST_BYTES)
            .read_line(&mut response)?;
        serde_json::from_str(&response).map_err(io::Error::other)
    }
}

#[derive(Debug)]
pub struct SingleInstance {
    _lock: File,
}

impl SingleInstance {
    pub fn acquire(data_directory: impl AsRef<Path>) -> Result<Self, AppError> {
        fs::create_dir_all(data_directory.as_ref()).map_err(platform_io_error)?;
        let path = data_directory.as_ref().join("verge.lock");
        let lock = File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(platform_io_error)?;
        lock.try_lock_exclusive().map_err(|error| {
            AppError::new(
                ErrorCode::Conflict,
                format!("another Verge instance is already running: {error}"),
            )
        })?;
        Ok(Self { _lock: lock })
    }
}

pub struct MacKeychain<R> {
    runner: R,
    service: String,
}

impl<R: CommandRunner> MacKeychain<R> {
    pub fn new(runner: R, service: impl Into<String>) -> Result<Self, AppError> {
        let service = service.into();
        validate_argument("keychain service", &service)?;
        Ok(Self { runner, service })
    }

    pub fn set(&mut self, account: &str, secret: &str) -> Result<(), AppError> {
        validate_argument("keychain account", account)?;
        validate_secret(secret)?;
        self.runner.run(
            SECURITY,
            &strings(&[
                "add-generic-password",
                "-U",
                "-a",
                account,
                "-s",
                &self.service,
                "-w",
                secret,
            ]),
        )?;
        Ok(())
    }

    pub fn get(&mut self, account: &str) -> Result<String, AppError> {
        validate_argument("keychain account", account)?;
        self.runner
            .run(
                SECURITY,
                &strings(&[
                    "find-generic-password",
                    "-a",
                    account,
                    "-s",
                    &self.service,
                    "-w",
                ]),
            )
            .map(|value| value.trim_end_matches(['\r', '\n']).to_owned())
    }

    pub fn delete(&mut self, account: &str) -> Result<(), AppError> {
        validate_argument("keychain account", account)?;
        self.runner.run(
            SECURITY,
            &strings(&[
                "delete-generic-password",
                "-a",
                account,
                "-s",
                &self.service,
            ]),
        )?;
        Ok(())
    }
}

pub struct MacNotifier<R> {
    runner: R,
}

impl<R: CommandRunner> MacNotifier<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn notify(&mut self, title: &str, body: &str) -> Result<(), AppError> {
        validate_argument("notification title", title)?;
        validate_argument("notification body", body)?;
        const SCRIPT: &str = "on run argv\n display notification (item 2 of argv) with title (item 1 of argv)\nend run";
        self.runner
            .run(OSASCRIPT, &strings(&["-e", SCRIPT, "--", title, body]))?;
        Ok(())
    }
}

fn validate_argument(name: &str, value: &str) -> Result<(), AppError> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!("{name} must be non-empty and contain no control characters"),
        ));
    }
    Ok(())
}

fn validate_secret(secret: &str) -> Result<(), AppError> {
    if secret.is_empty() || secret.contains('\0') {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "keychain secret must be non-empty and contain no NUL byte",
        ));
    }
    Ok(())
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub trait CommandRunner {
    fn run(&mut self, program: &str, args: &[String]) -> Result<String, AppError>;
}

/// Application-facing system proxy port. Platform implementations keep their
/// native command and recovery details behind this interface.
pub trait SystemProxyPlatform {
    fn state(&mut self, services: &[String]) -> Result<SystemProxyState, AppError>;
    fn recovery_pending(&self) -> bool;
    fn list_network_services(&mut self) -> Result<Vec<String>, AppError>;
    fn enable(
        &mut self,
        services: &[String],
        endpoint: &ProxyEndpoint,
    ) -> Result<SystemProxyState, AppError>;
    fn disable(&mut self) -> Result<SystemProxyState, AppError>;
    fn recover_pending(&mut self) -> Result<SystemProxyState, AppError>;
    fn set_socks(
        &mut self,
        services: &[String],
        enabled: bool,
        endpoint: &ProxyEndpoint,
    ) -> Result<SystemProxyState, AppError>;
    fn set_auto_proxy(
        &mut self,
        services: &[String],
        url: Option<&str>,
    ) -> Result<SystemProxyState, AppError>;
    fn set_bypass(
        &mut self,
        services: &[String],
        domains: &[String],
    ) -> Result<SystemProxyState, AppError>;
}

#[derive(Default)]
pub struct ProcessRunner;

impl CommandRunner for ProcessRunner {
    fn run(&mut self, program: &str, args: &[String]) -> Result<String, AppError> {
        let output = Command::new(program)
            .args(args)
            .output()
            .map_err(platform_io_error)?;
        if !output.status.success() {
            return Err(platform_error(
                String::from_utf8_lossy(&output.stderr).trim(),
            ));
        }
        String::from_utf8(output.stdout).map_err(platform_error)
    }
}

pub struct MacSystemProxy<R> {
    runner: R,
    recovery_path: PathBuf,
}

impl<R: CommandRunner> MacSystemProxy<R> {
    pub fn new(runner: R, recovery_path: impl Into<PathBuf>) -> Self {
        Self {
            runner,
            recovery_path: recovery_path.into(),
        }
    }

    pub fn state(&mut self, services: &[String]) -> Result<SystemProxyState, AppError> {
        validate_services(services)?;
        let mut states = Vec::with_capacity(services.len());
        for service in services {
            states.push(self.snapshot(service)?);
        }
        Ok(SystemProxyState {
            services: states,
            recovery_pending: self.recovery_path.is_file(),
        })
    }

    pub fn recovery_pending(&self) -> bool {
        self.recovery_path.is_file()
    }

    pub fn list_network_services(&mut self) -> Result<Vec<String>, AppError> {
        let output = self
            .runner
            .run(NETWORK_SETUP, &["-listallnetworkservices".into()])?;
        let services = output
            .lines()
            .map(str::trim)
            .filter(|line| {
                !line.is_empty() && !line.starts_with("An asterisk") && !line.starts_with('*')
            })
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if services.is_empty() {
            return Err(platform_error("no enabled macOS network services found"));
        }
        Ok(services)
    }

    pub fn enable(
        &mut self,
        services: &[String],
        endpoint: &ProxyEndpoint,
    ) -> Result<SystemProxyState, AppError> {
        ProxyEndpoint::new(&endpoint.host, endpoint.port)?;
        self.apply_change(services, |proxy, service| {
            proxy.apply_endpoint(service, endpoint)
        })
    }

    pub fn disable(&mut self) -> Result<SystemProxyState, AppError> {
        self.recover_pending()
    }

    pub fn recover_pending(&mut self) -> Result<SystemProxyState, AppError> {
        let previous = read_recovery(&self.recovery_path)?;
        self.restore_services(&previous.services)?;
        remove_recovery(&self.recovery_path)?;
        self.state(
            &previous
                .services
                .iter()
                .map(|state| state.service.clone())
                .collect::<Vec<_>>(),
        )
    }

    pub fn set_socks(
        &mut self,
        services: &[String],
        enabled: bool,
        endpoint: &ProxyEndpoint,
    ) -> Result<SystemProxyState, AppError> {
        ProxyEndpoint::new(&endpoint.host, endpoint.port)?;
        self.apply_change(services, |proxy, service| {
            proxy.set_protocol(
                "socksfirewall",
                service,
                &ProxyProtocolState {
                    enabled,
                    endpoint: endpoint.clone(),
                },
            )
        })
    }

    pub fn set_auto_proxy(
        &mut self,
        services: &[String],
        url: Option<&str>,
    ) -> Result<SystemProxyState, AppError> {
        if let Some(url) = url {
            validate_argument("auto proxy url", url)?;
        }
        let state = AutoProxyState {
            enabled: url.is_some(),
            url: url.map(str::to_owned),
        };
        self.apply_change(services, |proxy, service| {
            proxy.apply_auto_proxy(service, &state)
        })
    }

    pub fn set_bypass(
        &mut self,
        services: &[String],
        domains: &[String],
    ) -> Result<SystemProxyState, AppError> {
        for domain in domains {
            validate_argument("proxy bypass domain", domain)?;
        }
        self.apply_change(services, |proxy, service| {
            proxy.apply_bypass(service, domains)
        })
    }

    /// 快照当前状态，逐个网络服务应用变更，失败时按快照回滚。
    /// 恢复记录只在不存在时写入（记录永远是首个变更前的用户原始状态），
    /// 也只有本调用创建它时才会在回滚成功后移除。
    fn apply_change(
        &mut self,
        services: &[String],
        mut apply: impl FnMut(&mut Self, &str) -> Result<(), AppError>,
    ) -> Result<SystemProxyState, AppError> {
        validate_services(services)?;
        let previous = self.state(services)?;
        let created_record = !self.recovery_path.exists();
        if created_record {
            write_recovery(&self.recovery_path, &previous)?;
        }
        for service in services {
            if let Err(cause) = apply(self, service) {
                return match self.restore_services(&previous.services) {
                    Ok(()) => {
                        if created_record {
                            remove_recovery(&self.recovery_path)?;
                        }
                        Err(cause)
                    }
                    Err(rollback) => Err(platform_error(format!(
                        "system proxy apply failed: {cause}; rollback failed: {rollback}"
                    ))),
                };
            }
        }
        self.state(services)
    }

    fn snapshot(&mut self, service: &str) -> Result<SystemProxyServiceState, AppError> {
        Ok(SystemProxyServiceState {
            service: service.to_owned(),
            web: self.get_protocol("-getwebproxy", service)?,
            secure_web: self.get_protocol("-getsecurewebproxy", service)?,
            socks: self.get_protocol("-getsocksfirewallproxy", service)?,
            auto_proxy: self.get_auto_proxy(service)?,
            bypass: self.get_bypass(service)?,
        })
    }

    fn get_protocol(
        &mut self,
        action: &str,
        service: &str,
    ) -> Result<ProxyProtocolState, AppError> {
        let output = self
            .runner
            .run(NETWORK_SETUP, &[action.to_owned(), service.to_owned()])?;
        parse_protocol_state(&output)
    }

    fn get_auto_proxy(&mut self, service: &str) -> Result<AutoProxyState, AppError> {
        let output = self.runner.run(
            NETWORK_SETUP,
            &["-getautoproxyurl".to_owned(), service.to_owned()],
        )?;
        parse_auto_proxy_state(&output)
    }

    fn get_bypass(&mut self, service: &str) -> Result<Vec<String>, AppError> {
        let output = self.runner.run(
            NETWORK_SETUP,
            &["-getproxybypassdomains".to_owned(), service.to_owned()],
        )?;
        Ok(output
            .lines()
            .map(str::trim)
            .filter(|line| {
                !line.is_empty()
                    && !line.starts_with("There aren't any bypass domains")
                    && !line.starts_with("There are no bypass domains")
            })
            .map(str::to_owned)
            .collect())
    }

    fn apply_endpoint(&mut self, service: &str, endpoint: &ProxyEndpoint) -> Result<(), AppError> {
        let state = ProxyProtocolState {
            enabled: true,
            endpoint: endpoint.clone(),
        };
        self.set_protocol("web", service, &state)?;
        self.set_protocol("secureweb", service, &state)
    }

    fn restore_services(&mut self, states: &[SystemProxyServiceState]) -> Result<(), AppError> {
        let mut errors = Vec::new();
        for state in states {
            if let Err(error) = self.set_protocol("web", &state.service, &state.web) {
                errors.push(format!("{} web: {error}", state.service));
            }
            if let Err(error) = self.set_protocol("secureweb", &state.service, &state.secure_web) {
                errors.push(format!("{} secure web: {error}", state.service));
            }
            if let Err(error) = self.set_protocol("socksfirewall", &state.service, &state.socks) {
                errors.push(format!("{} socks: {error}", state.service));
            }
            if let Err(error) = self.apply_auto_proxy(&state.service, &state.auto_proxy) {
                errors.push(format!("{} automatic proxy: {error}", state.service));
            }
            if let Err(error) = self.apply_bypass(&state.service, &state.bypass) {
                errors.push(format!("{} bypass domains: {error}", state.service));
            }
        }
        if !errors.is_empty() {
            return Err(platform_error(format!(
                "system proxy recovery was incomplete: {}",
                errors.join("; ")
            )));
        }
        Ok(())
    }

    fn set_protocol(
        &mut self,
        protocol: &str,
        service: &str,
        state: &ProxyProtocolState,
    ) -> Result<(), AppError> {
        self.runner.run(
            NETWORK_SETUP,
            &[
                format!("-set{protocol}proxy"),
                service.to_owned(),
                state.endpoint.host.clone(),
                state.endpoint.port.to_string(),
            ],
        )?;
        self.runner.run(
            NETWORK_SETUP,
            &[
                format!("-set{protocol}proxystate"),
                service.to_owned(),
                if state.enabled { "on" } else { "off" }.to_owned(),
            ],
        )?;
        Ok(())
    }

    fn apply_auto_proxy(&mut self, service: &str, state: &AutoProxyState) -> Result<(), AppError> {
        if let Some(url) = state.url.as_deref() {
            validate_argument("auto proxy url", url)?;
            self.runner.run(
                NETWORK_SETUP,
                &["-setautoproxyurl".into(), service.into(), url.into()],
            )?;
        }
        self.runner.run(
            NETWORK_SETUP,
            &[
                "-setautoproxystate".into(),
                service.into(),
                if state.enabled { "on" } else { "off" }.into(),
            ],
        )?;
        Ok(())
    }

    fn apply_bypass(&mut self, service: &str, domains: &[String]) -> Result<(), AppError> {
        for domain in domains {
            validate_argument("proxy bypass domain", domain)?;
        }
        let mut args = vec!["-setproxybypassdomains".into(), service.into()];
        // networksetup 约定用字面量 Empty 清空绕过列表。
        if domains.is_empty() {
            args.push("Empty".into());
        } else {
            args.extend_from_slice(domains);
        }
        self.runner.run(NETWORK_SETUP, &args)?;
        Ok(())
    }
}

impl<R: CommandRunner> SystemProxyPlatform for MacSystemProxy<R> {
    fn state(&mut self, services: &[String]) -> Result<SystemProxyState, AppError> {
        MacSystemProxy::state(self, services)
    }

    fn recovery_pending(&self) -> bool {
        MacSystemProxy::recovery_pending(self)
    }

    fn list_network_services(&mut self) -> Result<Vec<String>, AppError> {
        MacSystemProxy::list_network_services(self)
    }

    fn enable(
        &mut self,
        services: &[String],
        endpoint: &ProxyEndpoint,
    ) -> Result<SystemProxyState, AppError> {
        MacSystemProxy::enable(self, services, endpoint)
    }

    fn disable(&mut self) -> Result<SystemProxyState, AppError> {
        MacSystemProxy::disable(self)
    }

    fn recover_pending(&mut self) -> Result<SystemProxyState, AppError> {
        MacSystemProxy::recover_pending(self)
    }

    fn set_socks(
        &mut self,
        services: &[String],
        enabled: bool,
        endpoint: &ProxyEndpoint,
    ) -> Result<SystemProxyState, AppError> {
        MacSystemProxy::set_socks(self, services, enabled, endpoint)
    }

    fn set_auto_proxy(
        &mut self,
        services: &[String],
        url: Option<&str>,
    ) -> Result<SystemProxyState, AppError> {
        MacSystemProxy::set_auto_proxy(self, services, url)
    }

    fn set_bypass(
        &mut self,
        services: &[String],
        domains: &[String],
    ) -> Result<SystemProxyState, AppError> {
        MacSystemProxy::set_bypass(self, services, domains)
    }
}

fn validate_services(services: &[String]) -> Result<(), AppError> {
    if services.is_empty()
        || services
            .iter()
            .any(|service| service.trim().is_empty() || service.chars().any(char::is_control))
    {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "at least one valid network service is required",
        ));
    }
    Ok(())
}

fn parse_protocol_state(output: &str) -> Result<ProxyProtocolState, AppError> {
    let mut enabled = None;
    let mut host = None;
    let mut port = None;
    for line in output.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key.trim() {
            "Enabled" => enabled = Some(value.trim().eq_ignore_ascii_case("yes")),
            "Server" => host = Some(value.trim().to_owned()),
            "Port" => port = value.trim().parse::<u16>().ok(),
            _ => {}
        }
    }
    Ok(ProxyProtocolState {
        enabled: enabled.ok_or_else(|| platform_error("missing proxy Enabled field"))?,
        endpoint: ProxyEndpoint::new(
            host.ok_or_else(|| platform_error("missing proxy Server field"))?,
            port.ok_or_else(|| platform_error("missing proxy Port field"))?,
        )?,
    })
}

fn parse_auto_proxy_state(output: &str) -> Result<AutoProxyState, AppError> {
    let mut enabled = None;
    let mut url = None;
    for line in output.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        match key.trim() {
            "Enabled" => enabled = Some(value.trim().eq_ignore_ascii_case("yes")),
            "URL" => {
                let value = value.trim();
                if !value.is_empty() && value != "(null)" {
                    url = Some(value.to_owned());
                }
            }
            _ => {}
        }
    }
    Ok(AutoProxyState {
        enabled: enabled.ok_or_else(|| platform_error("missing automatic proxy Enabled field"))?,
        url,
    })
}

fn write_recovery(path: &Path, state: &SystemProxyState) -> Result<(), AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| platform_error("recovery path has no parent"))?;
    fs::create_dir_all(parent).map_err(storage_error)?;
    let temporary = path.with_extension("tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(state).map_err(storage_error)?,
    )
    .map_err(storage_error)?;
    fs::rename(temporary, path).map_err(storage_error)
}

fn read_recovery(path: &Path) -> Result<SystemProxyState, AppError> {
    if !path.is_file() {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "no pending system proxy recovery record",
        ));
    }
    serde_json::from_slice(&fs::read(path).map_err(storage_error)?).map_err(storage_error)
}

fn remove_recovery(path: &Path) -> Result<(), AppError> {
    fs::remove_file(path).map_err(storage_error)
}

fn platform_io_error(error: io::Error) -> AppError {
    platform_error(error)
}

fn platform_error(error: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::PlatformFailed, error.to_string())
}

fn storage_error(error: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::StorageFailed, error.to_string())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[derive(Default)]
    struct FakeRunner {
        outputs: VecDeque<Result<String, AppError>>,
        calls: Vec<Vec<String>>,
    }

    impl CommandRunner for FakeRunner {
        fn run(&mut self, program: &str, args: &[String]) -> Result<String, AppError> {
            let mut call = vec![program.to_owned()];
            call.extend_from_slice(args);
            self.calls.push(call);
            self.outputs
                .pop_front()
                .unwrap_or_else(|| Ok(String::new()))
        }
    }

    struct TestDir(PathBuf);

    static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    impl TestDir {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "verge-platform-{}-{nonce}-{}",
                std::process::id(),
                TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn single_instance_lock_is_exclusive_and_released_on_drop() {
        let directory = TestDir::new();
        let first = SingleInstance::acquire(&directory.0).unwrap();
        let error = SingleInstance::acquire(&directory.0).unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        drop(first);
        SingleInstance::acquire(&directory.0).unwrap();
    }

    #[test]
    fn helper_client_distinguishes_ready_and_not_installed() {
        use std::os::unix::net::UnixListener;

        let directory = TestDir::new();
        let socket = directory.0.join("helper.sock");
        assert_eq!(
            MacHelperClient::new(&socket).status(),
            HelperStatus::NotInstalled
        );
        let listener = UnixListener::bind(&socket).unwrap();
        // SAFETY: getuid has no preconditions.
        let uid = unsafe { libc::getuid() };
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            verge_helper::serve_connection(stream, uid).unwrap();
        });
        assert_eq!(
            MacHelperClient::new(&socket).status(),
            HelperStatus::Ready {
                protocol_version: PROTOCOL_VERSION,
            }
        );
        server.join().unwrap();
    }

    #[test]
    fn keychain_uses_argumentized_security_commands() {
        let mut runner = FakeRunner::default();
        runner.outputs.push_back(Ok("stored-secret\n".into()));
        runner.outputs.push_back(Ok(String::new()));
        runner.outputs.push_back(Ok(String::new()));
        let mut keychain = MacKeychain::new(runner, "com.zzzgydi.verge").unwrap();
        assert_eq!(keychain.get("openai").unwrap(), "stored-secret");
        keychain.set("openai", "new-secret").unwrap();
        keychain.delete("openai").unwrap();
        assert_eq!(keychain.runner.calls.len(), 3);
        assert!(
            keychain
                .runner
                .calls
                .iter()
                .all(|call| call.first().is_some_and(|program| program == SECURITY))
        );
        assert_eq!(
            MacKeychain::new(FakeRunner::default(), "bad\nservice")
                .err()
                .unwrap()
                .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn notifications_pass_dynamic_text_as_argv() {
        let mut runner = FakeRunner::default();
        runner.outputs.push_back(Ok(String::new()));
        let mut notifier = MacNotifier::new(runner);
        notifier.notify("Verge", "Diagnostics exported").unwrap();
        let call = &notifier.runner.calls[0];
        assert_eq!(call[0], OSASCRIPT);
        assert_eq!(&call[4..], ["Verge", "Diagnostics exported"]);
        assert_eq!(
            notifier.notify("Verge", "bad\nbody").unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }

    #[derive(Default)]
    struct FakeLoginItem {
        enabled: bool,
        calls: Vec<&'static str>,
        failure: Option<AppError>,
    }

    impl LoginItemService for FakeLoginItem {
        fn status(&mut self) -> Result<bool, AppError> {
            self.calls.push("status");
            match &self.failure {
                Some(error) => Err(error.clone()),
                None => Ok(self.enabled),
            }
        }

        fn set_enabled(&mut self, enabled: bool) -> Result<(), AppError> {
            self.calls.push(if enabled { "enable" } else { "disable" });
            match self.failure.take() {
                Some(error) => Err(error),
                None => {
                    self.enabled = enabled;
                    Ok(())
                }
            }
        }
    }

    #[test]
    fn login_item_service_fake_records_sequence_and_maps_errors() {
        let mut item = FakeLoginItem::default();
        assert!(!item.status().unwrap());
        item.set_enabled(true).unwrap();
        assert!(item.status().unwrap());
        item.set_enabled(false).unwrap();
        assert_eq!(item.calls, ["status", "enable", "status", "disable"]);

        item.failure = Some(AppError::new(ErrorCode::PlatformFailed, "not a bundled .app"));
        let error = item.set_enabled(true).unwrap_err();
        assert_eq!(error.code, ErrorCode::PlatformFailed);
        assert!(error.message.contains("not a bundled .app"));
        assert!(!item.status().unwrap());
    }

    #[test]
    fn default_login_item_service_is_constructible() {
        let mut service = default_login_item_service();
        // 只验证对象可构造；真实系统交互留待人工冒烟。
        let _ = &mut service;
    }

    fn output(enabled: bool, host: &str, port: u16) -> String {
        format!(
            "Enabled: {}\nServer: {host}\nPort: {port}\nAuthenticated Proxy Enabled: 0\n",
            if enabled { "Yes" } else { "No" }
        )
    }

    fn snapshot_outputs(runner: &mut FakeRunner, services: usize) {
        for _ in 0..services {
            runner
                .outputs
                .push_back(Ok(output(false, "old.local", 8080)));
            runner
                .outputs
                .push_back(Ok(output(true, "secure.local", 8443)));
            runner
                .outputs
                .push_back(Ok(output(false, "socks.local", 1080)));
            runner
                .outputs
                .push_back(Ok("URL: (null)\nEnabled: No\n".into()));
            runner
                .outputs
                .push_back(Ok("There aren't any bypass domains set on Wi-Fi.\n".into()));
        }
    }

    #[test]
    fn reads_multiple_services_without_shell_commands() {
        let directory = TestDir::new();
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 2);
        let mut proxy = MacSystemProxy::new(runner, directory.0.join("recovery.json"));
        let state = proxy.state(&["Wi-Fi".into(), "Ethernet".into()]).unwrap();
        assert_eq!(state.services.len(), 2);
        assert_eq!(proxy.runner.calls[0][0], NETWORK_SETUP);
        assert_eq!(proxy.runner.calls[0][2], "Wi-Fi");
        assert!(!state.recovery_pending);
    }

    #[test]
    fn discovers_enabled_network_services_and_skips_disabled_entries() {
        let directory = TestDir::new();
        let mut runner = FakeRunner::default();
        runner.outputs.push_back(Ok(
            "An asterisk (*) denotes that a network service is disabled.\nWi-Fi\n*Ethernet\nUSB 10/100/1000 LAN\n"
                .into(),
        ));
        let mut proxy = MacSystemProxy::new(runner, directory.0.join("recovery.json"));
        assert_eq!(
            proxy.list_network_services().unwrap(),
            ["Wi-Fi", "USB 10/100/1000 LAN"]
        );
        assert_eq!(
            proxy.runner.calls[0],
            [NETWORK_SETUP, "-listallnetworkservices"]
        );
    }

    #[test]
    fn enable_persists_snapshot_before_mutation_and_recovers_it() {
        let directory = TestDir::new();
        let recovery = directory.0.join("recovery.json");
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 1);
        for _ in 0..4 {
            runner.outputs.push_back(Ok(String::new()));
        }
        snapshot_outputs(&mut runner, 1);
        for _ in 0..8 {
            runner.outputs.push_back(Ok(String::new()));
        }
        snapshot_outputs(&mut runner, 1);
        let mut proxy = MacSystemProxy::new(runner, &recovery);
        let services = ["Wi-Fi".into()];
        let enabled = proxy
            .enable(&services, &ProxyEndpoint::new("127.0.0.1", 7890).unwrap())
            .unwrap();
        assert!(enabled.recovery_pending);
        assert!(recovery.is_file());
        let restored = proxy.recover_pending().unwrap();
        assert!(!restored.recovery_pending);
        assert!(!recovery.exists());
        assert!(
            proxy
                .runner
                .calls
                .iter()
                .any(|call| call.get(1).is_some_and(|arg| arg == "-setwebproxy"))
        );
    }

    #[test]
    fn partial_failure_rolls_back_and_removes_recovery_record() {
        let directory = TestDir::new();
        let recovery = directory.0.join("recovery.json");
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 1);
        runner.outputs.push_back(Ok(String::new()));
        runner.outputs.push_back(Ok(String::new()));
        runner
            .outputs
            .push_back(Err(platform_error("secure proxy failed")));
        let mut proxy = MacSystemProxy::new(runner, &recovery);
        let error = proxy
            .enable(
                &["Wi-Fi".into()],
                &ProxyEndpoint::new("127.0.0.1", 7890).unwrap(),
            )
            .unwrap_err();
        assert!(error.message.contains("secure proxy failed"));
        assert!(!recovery.exists());
        assert_eq!(proxy.runner.calls.len(), 16);
    }

    #[test]
    fn failed_recovery_keeps_record_for_next_start() {
        let directory = TestDir::new();
        let recovery = directory.0.join("recovery.json");
        let previous = SystemProxyState {
            services: vec![SystemProxyServiceState {
                service: "Wi-Fi".into(),
                web: ProxyProtocolState {
                    enabled: false,
                    endpoint: ProxyEndpoint::new("old.local", 8080).unwrap(),
                },
                secure_web: ProxyProtocolState {
                    enabled: false,
                    endpoint: ProxyEndpoint::new("old.local", 8080).unwrap(),
                },
                socks: ProxyProtocolState {
                    enabled: false,
                    endpoint: ProxyEndpoint::new("old.local", 1080).unwrap(),
                },
                auto_proxy: AutoProxyState {
                    enabled: false,
                    url: None,
                },
                bypass: Vec::new(),
            }],
            recovery_pending: false,
        };
        write_recovery(&recovery, &previous).unwrap();
        let mut runner = FakeRunner::default();
        runner
            .outputs
            .push_back(Err(platform_error("permission denied")));
        let mut proxy = MacSystemProxy::new(runner, &recovery);
        assert!(proxy.recover_pending().is_err());
        assert!(recovery.is_file());
        assert_eq!(proxy.runner.calls.len(), 7);
    }

    #[test]
    fn snapshot_reads_socks_pac_and_bypass() {
        let directory = TestDir::new();
        let mut runner = FakeRunner::default();
        runner.outputs.push_back(Ok(output(false, "old.local", 8080)));
        runner
            .outputs
            .push_back(Ok(output(true, "secure.local", 8443)));
        runner
            .outputs
            .push_back(Ok(output(true, "socks.local", 1080)));
        runner
            .outputs
            .push_back(Ok("URL: http://127.0.0.1/proxy.pac\nEnabled: Yes\n".into()));
        runner
            .outputs
            .push_back(Ok("*.local\n192.168.0.0/16\n".into()));
        let mut proxy = MacSystemProxy::new(runner, directory.0.join("recovery.json"));
        let state = proxy.state(&["Wi-Fi".into()]).unwrap();
        let service = &state.services[0];
        assert_eq!(
            service.socks.endpoint,
            ProxyEndpoint::new("socks.local", 1080).unwrap()
        );
        assert!(service.socks.enabled);
        assert_eq!(
            service.auto_proxy.url.as_deref(),
            Some("http://127.0.0.1/proxy.pac")
        );
        assert!(service.auto_proxy.enabled);
        assert_eq!(service.bypass, ["*.local", "192.168.0.0/16"]);
        let calls = &proxy.runner.calls;
        assert_eq!(calls[2][1], "-getsocksfirewallproxy");
        assert_eq!(calls[3][1], "-getautoproxyurl");
        assert_eq!(calls[4][1], "-getproxybypassdomains");
    }

    #[test]
    fn set_socks_uses_argumentized_commands_and_recovers() {
        let directory = TestDir::new();
        let recovery = directory.0.join("recovery.json");
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 1);
        for _ in 0..2 {
            runner.outputs.push_back(Ok(String::new()));
        }
        snapshot_outputs(&mut runner, 1);
        for _ in 0..8 {
            runner.outputs.push_back(Ok(String::new()));
        }
        snapshot_outputs(&mut runner, 1);
        let mut proxy = MacSystemProxy::new(runner, &recovery);
        let services = ["Wi-Fi".into()];
        let state = proxy
            .set_socks(&services, true, &ProxyEndpoint::new("127.0.0.1", 7891).unwrap())
            .unwrap();
        assert!(state.recovery_pending);
        let calls = &proxy.runner.calls;
        assert_eq!(
            calls[5],
            [
                NETWORK_SETUP,
                "-setsocksfirewallproxy",
                "Wi-Fi",
                "127.0.0.1",
                "7891"
            ]
        );
        assert_eq!(
            calls[6],
            [NETWORK_SETUP, "-setsocksfirewallproxystate", "Wi-Fi", "on"]
        );
        proxy.recover_pending().unwrap();
        assert!(!recovery.exists());
        assert!(
            proxy
                .runner
                .calls
                .iter()
                .any(|call| call.get(1).is_some_and(|arg| arg == "-setsocksfirewallproxy"))
        );
    }

    #[test]
    fn set_socks_partial_failure_rolls_back_and_removes_recovery_record() {
        let directory = TestDir::new();
        let recovery = directory.0.join("recovery.json");
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 1);
        runner
            .outputs
            .push_back(Err(platform_error("socks proxy failed")));
        let mut proxy = MacSystemProxy::new(runner, &recovery);
        let error = proxy
            .set_socks(
                &["Wi-Fi".into()],
                true,
                &ProxyEndpoint::new("127.0.0.1", 7891).unwrap(),
            )
            .unwrap_err();
        assert!(error.message.contains("socks proxy failed"));
        assert!(!recovery.exists());
        // 快照 5 次读取 + 1 次失败写入 + 8 次回滚写入。
        assert_eq!(proxy.runner.calls.len(), 14);
    }

    #[test]
    fn set_auto_proxy_enables_with_url_and_disables_without() {
        let directory = TestDir::new();
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 1);
        for _ in 0..2 {
            runner.outputs.push_back(Ok(String::new()));
        }
        snapshot_outputs(&mut runner, 1);
        snapshot_outputs(&mut runner, 1);
        runner.outputs.push_back(Ok(String::new()));
        snapshot_outputs(&mut runner, 1);
        let mut proxy = MacSystemProxy::new(runner, directory.0.join("recovery.json"));
        let services = ["Wi-Fi".into()];
        proxy
            .set_auto_proxy(&services, Some("http://127.0.0.1/proxy.pac"))
            .unwrap();
        let calls = &proxy.runner.calls;
        assert_eq!(
            calls[5],
            [
                NETWORK_SETUP,
                "-setautoproxyurl",
                "Wi-Fi",
                "http://127.0.0.1/proxy.pac"
            ]
        );
        assert_eq!(
            calls[6],
            [NETWORK_SETUP, "-setautoproxystate", "Wi-Fi", "on"]
        );
        proxy.set_auto_proxy(&services, None).unwrap();
        let calls = &proxy.runner.calls;
        assert_eq!(
            calls[17],
            [NETWORK_SETUP, "-setautoproxystate", "Wi-Fi", "off"]
        );
        assert_eq!(
            proxy
                .set_auto_proxy(&services, Some("http://bad\nurl"))
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn set_bypass_passes_domains_and_uses_empty_marker() {
        let directory = TestDir::new();
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 1);
        runner.outputs.push_back(Ok(String::new()));
        snapshot_outputs(&mut runner, 1);
        snapshot_outputs(&mut runner, 1);
        runner.outputs.push_back(Ok(String::new()));
        snapshot_outputs(&mut runner, 1);
        let mut proxy = MacSystemProxy::new(runner, directory.0.join("recovery.json"));
        let services = ["Wi-Fi".into()];
        proxy
            .set_bypass(&services, &["*.local".into(), "192.168.0.0/16".into()])
            .unwrap();
        assert_eq!(
            proxy.runner.calls[5],
            [
                NETWORK_SETUP,
                "-setproxybypassdomains",
                "Wi-Fi",
                "*.local",
                "192.168.0.0/16"
            ]
        );
        proxy.set_bypass(&services, &[]).unwrap();
        assert_eq!(
            proxy.runner.calls[16],
            [NETWORK_SETUP, "-setproxybypassdomains", "Wi-Fi", "Empty"]
        );
        assert_eq!(
            proxy
                .set_bypass(&services, &["bad\ndomain".into()])
                .unwrap_err()
                .code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn later_mutations_do_not_clobber_the_original_recovery_record() {
        let directory = TestDir::new();
        let recovery = directory.0.join("recovery.json");
        let mut runner = FakeRunner::default();
        snapshot_outputs(&mut runner, 1);
        for _ in 0..4 {
            runner.outputs.push_back(Ok(String::new()));
        }
        snapshot_outputs(&mut runner, 1);
        snapshot_outputs(&mut runner, 1);
        runner.outputs.push_back(Ok(String::new()));
        snapshot_outputs(&mut runner, 1);
        let mut proxy = MacSystemProxy::new(runner, &recovery);
        let services = ["Wi-Fi".into()];
        proxy
            .enable(&services, &ProxyEndpoint::new("127.0.0.1", 7890).unwrap())
            .unwrap();
        let original = fs::read(&recovery).unwrap();
        proxy.set_bypass(&services, &["*.local".into()]).unwrap();
        assert_eq!(fs::read(&recovery).unwrap(), original);
    }
}
