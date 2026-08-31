use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::Ipv4Addr,
    os::unix::{fs::PermissionsExt, net::UnixStream},
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

pub use verge_helper_protocol::{
    HelperRequest, HelperResponse, MAX_REQUEST_BYTES, PROTOCOL_VERSION, TunConfig,
};

/// helper 内部的结构化错误，按 code 映射为协议 Error 响应。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HelperFailure {
    pub code: &'static str,
    pub message: String,
}

impl HelperFailure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// TUN 生命周期后端抽象：真实实现只在 helper 进程内以 root 运行，
/// 测试注入 fake，不触达真实系统。
pub trait TunBackend: Send {
    /// 当前进程是否真的能执行 TUN 特权操作（用于如实上报 Capabilities）。
    fn tun_lifecycle_supported(&self) -> bool;
    /// 创建并配置 utun 设备，返回实际设备名（如 `utun4`）。
    /// 入参已在校验入口完成逐字段校验。
    fn enable_tun(&mut self, config: &ValidatedTun) -> Result<String, HelperFailure>;
    /// 拆除指定设备；None 表示拆除本进程管理的全部设备。
    fn disable_tun(&mut self, device: Option<&str>) -> Result<(), HelperFailure>;
}

pub type SharedTunBackend = Arc<Mutex<dyn TunBackend>>;

pub fn shared_tun_backend(backend: impl TunBackend + 'static) -> SharedTunBackend {
    Arc::new(Mutex::new(backend))
}

pub fn serve_connection(
    mut stream: UnixStream,
    allowed_uid: u32,
    tun: &SharedTunBackend,
) -> std::io::Result<()> {
    let peer_uid = peer_uid(&stream)?;
    if peer_uid != allowed_uid && peer_uid != 0 {
        return write_response(
            &mut stream,
            &HelperResponse::Error {
                code: "unauthorized_peer".into(),
                message: "peer uid is not authorized".into(),
            },
        );
    }
    let mut line = String::new();
    BufReader::new(stream.try_clone()?)
        .take(MAX_REQUEST_BYTES)
        .read_line(&mut line)?;
    let request = match serde_json::from_str::<HelperRequest>(&line) {
        Ok(request) => request,
        Err(error) => {
            return write_response(
                &mut stream,
                &HelperResponse::Error {
                    code: "invalid_request".into(),
                    message: error.to_string(),
                },
            );
        }
    };
    let response = handle_request(request, tun);
    write_response(&mut stream, &response)
}

fn handle_request(request: HelperRequest, tun: &SharedTunBackend) -> HelperResponse {
    match request {
        HelperRequest::Ping { protocol_version } if protocol_version == PROTOCOL_VERSION => {
            HelperResponse::Pong { protocol_version }
        }
        HelperRequest::Ping { .. } => HelperResponse::Error {
            code: "protocol_mismatch".into(),
            message: format!("helper protocol version is {PROTOCOL_VERSION}"),
        },
        HelperRequest::GetCapabilities => HelperResponse::Capabilities {
            protocol_version: PROTOCOL_VERSION,
            tun_lifecycle: lock_tun(tun).tun_lifecycle_supported(),
        },
        HelperRequest::EnableTun { config } => {
            match validate_tun_config(&config)
                .map_err(validated_failure)
                .and_then(|validated| lock_tun(tun).enable_tun(&validated))
            {
                Ok(device) => HelperResponse::TunEnabled { device },
                Err(failure) => failure_response(failure),
            }
        }
        HelperRequest::DisableTun { device } => {
            let result = match &device {
                Some(device) => parse_utun_unit(device)
                    .map(|_| ())
                    .and_then(|()| lock_tun(tun).disable_tun(Some(device))),
                None => lock_tun(tun).disable_tun(None),
            };
            match result {
                Ok(()) => HelperResponse::TunDisabled,
                Err(failure) => failure_response(failure),
            }
        }
    }
}

fn lock_tun(tun: &SharedTunBackend) -> MutexGuard<'_, dyn TunBackend + 'static> {
    tun.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn failure_response(failure: HelperFailure) -> HelperResponse {
    HelperResponse::Error {
        code: failure.code.into(),
        message: failure.message,
    }
}

/// 校验过的 TUN 参数：IP/掩码/路由均已解析为结构化值，设备名限定 `utunN`。
/// 校验只在 serve 入口做一次，后端直接消费结构化结果。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedTun {
    unit: Option<u32>,
    address: Ipv4Addr,
    netmask: Ipv4Addr,
    mtu: u16,
    routes: Vec<(Ipv4Addr, u8)>,
}

fn validated_failure(message: String) -> HelperFailure {
    HelperFailure::new("invalid_tun_config", message)
}

/// 设备名只接受 `utunN`（0..=255），杜绝注入 ifconfig/route 的其它接口。
fn parse_utun_unit(device: &str) -> Result<u32, HelperFailure> {
    let invalid = || {
        HelperFailure::new(
            "invalid_tun_config",
            format!("device must be a utun interface, got '{device}'"),
        )
    };
    let unit = device
        .strip_prefix("utun")
        .ok_or_else(invalid)?
        .parse::<u32>()
        .map_err(|_| invalid())?;
    if unit > 255 {
        return Err(HelperFailure::new(
            "invalid_tun_config",
            format!("utun unit {unit} is out of range"),
        ));
    }
    Ok(unit)
}

fn validate_tun_config(config: &TunConfig) -> Result<ValidatedTun, String> {
    let unit = config
        .device
        .as_deref()
        .map(|device| parse_utun_unit(device).map_err(|failure| failure.message))
        .transpose()?;
    let address = parse_ipv4("address", &config.address)?;
    if address.is_unspecified() || address.is_multicast() || address.is_broadcast() {
        return Err(format!(
            "address {} is not a usable host address",
            config.address
        ));
    }
    let netmask = parse_ipv4("netmask", &config.netmask)?;
    // 掩码必须是连续的前缀（前导 1 的个数等于 1 的总数）。
    let bits = u32::from(netmask);
    if bits.count_ones() != bits.leading_ones() {
        return Err(format!("netmask {} is not contiguous", config.netmask));
    }
    if !(576..=9_000).contains(&config.mtu) {
        return Err(format!("mtu {} is out of range (576..=9000)", config.mtu));
    }
    if config.routes.len() > 32 {
        return Err(format!("too many routes: {} (max 32)", config.routes.len()));
    }
    let mut routes = Vec::with_capacity(config.routes.len());
    for route in &config.routes {
        let Some((network, prefix)) = route.split_once('/') else {
            return Err(format!("route '{route}' must be CIDR (e.g. 198.18.0.0/16)"));
        };
        let network = parse_ipv4("route network", network)?;
        let prefix = prefix
            .parse::<u8>()
            .ok()
            .filter(|prefix| *prefix <= 32)
            .ok_or_else(|| format!("route '{route}' has an invalid prefix length"))?;
        routes.push((network, prefix));
    }
    Ok(ValidatedTun {
        unit,
        address,
        netmask,
        mtu: config.mtu,
        routes,
    })
}

fn parse_ipv4(field: &str, value: &str) -> Result<Ipv4Addr, String> {
    value
        .parse::<Ipv4Addr>()
        .map_err(|_| format!("{field} '{value}' is not a valid IPv4 address"))
}

/// macOS 真实后端：以 PF_SYSTEM/SYSPROTO_CONTROL 创建 utun（需要 root），
/// 再用参数校验过的固定 ifconfig/route 命令集完成配置。
/// 只管理本进程创建的设备；进程持有的 fd 关闭即销毁设备。
#[derive(Default)]
pub struct MacTun {
    devices: HashMap<String, TunDevice>,
}

struct TunDevice {
    fd: i32,
}

impl Drop for TunDevice {
    fn drop(&mut self) {
        // SAFETY: fd 来自本对象拥有的 socket；HashMap 唯一所有权保证只 close 一次。
        unsafe { libc::close(self.fd) };
    }
}

impl TunBackend for MacTun {
    fn tun_lifecycle_supported(&self) -> bool {
        // utun 创建需要 root；非 root 运行时如实上报不支持。
        cfg!(target_os = "macos") && current_uid() == 0
    }

    fn enable_tun(&mut self, config: &ValidatedTun) -> Result<String, HelperFailure> {
        let created = create_utun(config.unit)?;
        if self.devices.contains_key(&created.name) {
            return Err(HelperFailure::new(
                "tun_conflict",
                format!("device '{}' is already managed by this helper", created.name),
            ));
        }
        if let Err(failure) = configure_utun(&created.name, config) {
            // 配置失败：drop 关闭 fd，销毁刚创建的设备，不留半成品。
            drop(created.into_device());
            return Err(failure);
        }
        let name = created.name.clone();
        self.devices.insert(name.clone(), created.into_device());
        Ok(name)
    }

    fn disable_tun(&mut self, device: Option<&str>) -> Result<(), HelperFailure> {
        match device {
            Some(device) => {
                if self.devices.remove(device).is_none() {
                    return Err(HelperFailure::new(
                        "tun_not_managed",
                        format!("device '{device}' is not managed by this helper"),
                    ));
                }
                Ok(())
            }
            None => {
                self.devices.clear();
                Ok(())
            }
        }
    }
}

struct CreatedUtun {
    fd: i32,
    name: String,
}

impl CreatedUtun {
    fn into_device(self) -> TunDevice {
        TunDevice { fd: self.fd }
    }
}

#[cfg(target_os = "macos")]
fn create_utun(unit: Option<u32>) -> Result<CreatedUtun, HelperFailure> {
    const PF_SYSTEM: i32 = 32;
    const SYSPROTO_CONTROL: i32 = 2;
    const AF_SYSTEM: u8 = 32;
    const AF_SYS_CONTROL: u16 = 2;
    // CTLIOCGINFO = _IOWR('N', 3, struct ctl_info)，sizeof(ctl_info) = 4 + 96 = 100。
    const CTLIOCGINFO: libc::c_ulong = 0xC064_4E03;
    const UTUN_OPT_IFNAME: i32 = 2;
    const UTUN_CONTROL_NAME: &[u8] = b"com.apple.net.utun_control\0";

    #[repr(C)]
    struct CtlInfo {
        ctl_id: u32,
        ctl_name: [libc::c_char; 96],
    }

    #[repr(C)]
    struct SockaddrCtl {
        sc_len: u8,
        sc_family: u8,
        ss_sysaddr: u16,
        sc_id: u32,
        sc_unit: u32,
        sc_reserved: [u32; 5],
    }

    // SAFETY: 参数均为合法常量；返回 fd 由本函数全权管理（失败路径立即 close）。
    let fd = unsafe { libc::socket(PF_SYSTEM, libc::SOCK_DGRAM, SYSPROTO_CONTROL) };
    if fd < 0 {
        return Err(HelperFailure::new(
            "tun_create_failed",
            format!(
                "utun control socket failed: {}",
                std::io::Error::last_os_error()
            ),
        ));
    }
    let cleanup = |fd: i32| {
        // SAFETY: fd 有效且仅此一处关闭。
        unsafe { libc::close(fd) };
    };

    let mut info = CtlInfo {
        ctl_id: 0,
        ctl_name: [0; 96],
    };
    // SAFETY: c_char 与 u8 等宽；源串带 NUL 结尾且长度 <= 96。
    info.ctl_name[..UTUN_CONTROL_NAME.len()]
        .copy_from_slice(unsafe { &*(UTUN_CONTROL_NAME as *const [u8] as *const [libc::c_char]) });
    // SAFETY: info 是有效栈缓冲区，布局与 CTLIOCGINFO 声明的 ctl_info 一致。
    if unsafe { libc::ioctl(fd, CTLIOCGINFO, &raw mut info) } != 0 {
        let error = std::io::Error::last_os_error();
        cleanup(fd);
        return Err(HelperFailure::new(
            "tun_create_failed",
            format!("utun control lookup failed: {error}"),
        ));
    }

    let candidates: Vec<u32> = match unit {
        Some(unit) => vec![unit],
        None => (0..=255).collect(),
    };
    let mut last_error = None;
    for candidate in candidates {
        let address = SockaddrCtl {
            sc_len: size_of::<SockaddrCtl>() as u8,
            sc_family: AF_SYSTEM,
            ss_sysaddr: AF_SYS_CONTROL,
            sc_id: info.ctl_id,
            // utun 单元号从 1 开始映射（utun0 对应 sc_unit 1）。
            sc_unit: candidate + 1,
            sc_reserved: [0; 5],
        };
        // SAFETY: address 是有效栈缓冲区，长度由 size_of 明确给出。
        let connected = unsafe {
            libc::connect(
                fd,
                &raw const address as *const libc::sockaddr,
                size_of::<SockaddrCtl>() as u32,
            )
        } == 0;
        if connected {
            let mut name = [0_u8; 64];
            let mut name_len = name.len() as libc::socklen_t;
            // SAFETY: name 缓冲区与传入长度一致。
            let ok = unsafe {
                libc::getsockopt(
                    fd,
                    SYSPROTO_CONTROL,
                    UTUN_OPT_IFNAME,
                    name.as_mut_ptr().cast(),
                    &raw mut name_len,
                )
            } == 0;
            if !ok {
                let error = std::io::Error::last_os_error();
                cleanup(fd);
                return Err(HelperFailure::new(
                    "tun_create_failed",
                    format!("utun interface name lookup failed: {error}"),
                ));
            }
            let name = String::from_utf8_lossy(
                &name[..(name_len as usize).saturating_sub(1).min(name.len())],
            )
            .into_owned();
            if parse_utun_unit(&name).is_err() {
                cleanup(fd);
                return Err(HelperFailure::new(
                    "tun_create_failed",
                    format!("kernel returned an unexpected utun name '{name}'"),
                ));
            }
            return Ok(CreatedUtun { fd, name });
        }
        last_error = Some(std::io::Error::last_os_error());
        if unit.is_some() {
            break;
        }
    }
    cleanup(fd);
    Err(HelperFailure::new(
        "tun_create_failed",
        format!(
            "no usable utun unit found: {}",
            last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "unknown error".into())
        ),
    ))
}

#[cfg(not(target_os = "macos"))]
fn create_utun(_unit: Option<u32>) -> Result<CreatedUtun, HelperFailure> {
    Err(HelperFailure::new(
        "tun_unsupported",
        "TUN lifecycle is only implemented on macOS",
    ))
}

/// 用固定命令模板配置设备：ifconfig 设地址/掩码/MTU/up，route 逐条挂路由。
/// 参数全部来自 ValidatedTun（结构化值），直接 exec、不经过 shell。
#[cfg(target_os = "macos")]
fn configure_utun(device: &str, config: &ValidatedTun) -> Result<(), HelperFailure> {
    run_fixed(
        "/sbin/ifconfig",
        &[
            device.to_owned(),
            "inet".into(),
            config.address.to_string(),
            config.address.to_string(),
            "netmask".into(),
            config.netmask.to_string(),
            "mtu".into(),
            config.mtu.to_string(),
            "up".into(),
        ],
    )?;
    for (network, prefix) in &config.routes {
        run_fixed(
            "/sbin/route",
            &[
                "-n".into(),
                "add".into(),
                "-inet".into(),
                format!("{network}/{prefix}"),
                "-interface".into(),
                device.to_owned(),
            ],
        )?;
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn configure_utun(_device: &str, _config: &ValidatedTun) -> Result<(), HelperFailure> {
    Err(HelperFailure::new(
        "tun_unsupported",
        "TUN lifecycle is only implemented on macOS",
    ))
}

#[cfg(target_os = "macos")]
fn run_fixed(program: &'static str, args: &[String]) -> Result<(), HelperFailure> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|error| {
            HelperFailure::new("tun_configure_failed", format!("{program}: {error}"))
        })?;
    if output.status.success() {
        Ok(())
    } else {
        Err(HelperFailure::new(
            "tun_configure_failed",
            format!(
                "{program} {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ))
    }
}

pub fn prepare_socket_path(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

pub fn restrict_socket(path: &Path) -> std::io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

pub fn current_uid() -> u32 {
    // SAFETY: getuid has no preconditions.
    unsafe { libc::getuid() }
}

fn write_response(stream: &mut UnixStream, response: &HelperResponse) -> std::io::Result<()> {
    serde_json::to_writer(&mut *stream, response)?;
    stream.write_all(b"\n")?;
    stream.flush()
}

#[cfg(target_os = "macos")]
fn peer_uid(stream: &UnixStream) -> std::io::Result<u32> {
    use std::os::fd::AsRawFd;

    let mut uid = 0;
    let mut gid = 0;
    // SAFETY: getpeereid writes to valid uid/gid pointers for this connected Unix socket fd.
    let result = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    if result == 0 {
        Ok(uid)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "macos"))]
fn peer_uid(_stream: &UnixStream) -> std::io::Result<u32> {
    current_uid()
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Write},
        os::unix::net::UnixStream,
        sync::{Arc, Mutex},
    };

    use super::*;

    struct FakeTun {
        supported: bool,
        calls: Arc<Mutex<Vec<String>>>,
        failure: Arc<Mutex<Option<HelperFailure>>>,
    }

    impl TunBackend for FakeTun {
        fn tun_lifecycle_supported(&self) -> bool {
            self.supported
        }

        fn enable_tun(&mut self, config: &ValidatedTun) -> Result<String, HelperFailure> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("enable:{:?}:{}", config.unit, config.address));
            match self.failure.lock().unwrap().take() {
                Some(failure) => Err(failure),
                None => Ok(config
                    .unit
                    .map(|unit| format!("utun{unit}"))
                    .unwrap_or_else(|| "utun4".into())),
            }
        }

        fn disable_tun(&mut self, device: Option<&str>) -> Result<(), HelperFailure> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("disable:{device:?}"));
            match self.failure.lock().unwrap().take() {
                Some(failure) => Err(failure),
                None => Ok(()),
            }
        }
    }

    struct FakeFixture {
        tun: SharedTunBackend,
        calls: Arc<Mutex<Vec<String>>>,
        failure: Arc<Mutex<Option<HelperFailure>>>,
    }

    fn fake_backend(supported: bool) -> FakeFixture {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let failure = Arc::new(Mutex::new(None));
        let tun = shared_tun_backend(FakeTun {
            supported,
            calls: calls.clone(),
            failure: failure.clone(),
        });
        FakeFixture {
            tun,
            calls,
            failure,
        }
    }

    fn exchange(
        request: &HelperRequest,
        allowed_uid: u32,
        tun: &SharedTunBackend,
    ) -> HelperResponse {
        let (mut client, server) = UnixStream::pair().unwrap();
        let tun = tun.clone();
        let handle =
            std::thread::spawn(move || serve_connection(server, allowed_uid, &tun).unwrap());
        serde_json::to_writer(&mut client, request).unwrap();
        client.write_all(b"\n").unwrap();
        let mut response = String::new();
        BufReader::new(client).read_line(&mut response).unwrap();
        handle.join().unwrap();
        serde_json::from_str(&response).unwrap()
    }

    #[test]
    fn ping_negotiates_an_exact_protocol_version() {
        let fixture = fake_backend(false);
        let uid = current_uid();
        assert_eq!(
            exchange(
                &HelperRequest::Ping {
                    protocol_version: PROTOCOL_VERSION,
                },
                uid,
                &fixture.tun,
            ),
            HelperResponse::Pong {
                protocol_version: PROTOCOL_VERSION,
            }
        );
        assert!(matches!(
            exchange(
                &HelperRequest::Ping {
                    protocol_version: PROTOCOL_VERSION + 1,
                },
                uid,
                &fixture.tun,
            ),
            HelperResponse::Error { code, .. } if code == "protocol_mismatch"
        ));
    }

    #[test]
    fn capabilities_report_what_the_backend_can_actually_do() {
        let unsupported = fake_backend(false);
        let supported = fake_backend(true);
        let uid = current_uid();
        assert_eq!(
            exchange(&HelperRequest::GetCapabilities, uid, &unsupported.tun),
            HelperResponse::Capabilities {
                protocol_version: PROTOCOL_VERSION,
                tun_lifecycle: false,
            }
        );
        assert_eq!(
            exchange(&HelperRequest::GetCapabilities, uid, &supported.tun),
            HelperResponse::Capabilities {
                protocol_version: PROTOCOL_VERSION,
                tun_lifecycle: true,
            }
        );
    }

    #[test]
    fn enable_and_disable_tun_round_trip_through_the_backend() {
        let fixture = fake_backend(true);
        let uid = current_uid();
        assert_eq!(
            exchange(
                &HelperRequest::EnableTun {
                    config: TunConfig::verge_default(),
                },
                uid,
                &fixture.tun,
            ),
            HelperResponse::TunEnabled {
                device: "utun4".into()
            }
        );
        assert_eq!(
            exchange(&HelperRequest::DisableTun { device: None }, uid, &fixture.tun),
            HelperResponse::TunDisabled
        );
        assert_eq!(
            fixture.calls.lock().unwrap().as_slice(),
            [
                "enable:None:198.18.0.1".to_owned(),
                "disable:None".to_owned()
            ]
        );
    }

    #[test]
    fn invalid_tun_configs_are_rejected_before_touching_the_backend() {
        let fixture = fake_backend(true);
        let uid = current_uid();
        for config in [
            TunConfig {
                device: Some("eth0".into()),
                ..TunConfig::verge_default()
            },
            TunConfig {
                address: "999.0.0.1".into(),
                ..TunConfig::verge_default()
            },
            TunConfig {
                address: "0.0.0.0".into(),
                ..TunConfig::verge_default()
            },
            TunConfig {
                netmask: "255.0.255.0".into(),
                ..TunConfig::verge_default()
            },
            TunConfig {
                mtu: 100,
                ..TunConfig::verge_default()
            },
            TunConfig {
                routes: vec!["198.18.0.0".into()],
                ..TunConfig::verge_default()
            },
            TunConfig {
                routes: vec!["198.18.0.0/33".into()],
                ..TunConfig::verge_default()
            },
            TunConfig {
                routes: (0..33).map(|index| format!("10.0.0.{index}/32")).collect(),
                ..TunConfig::verge_default()
            },
        ] {
            assert!(
                matches!(
                    exchange(&HelperRequest::EnableTun { config }, uid, &fixture.tun),
                    HelperResponse::Error { code, .. } if code == "invalid_tun_config"
                ),
                "config must be rejected"
            );
        }
        assert!(fixture.calls.lock().unwrap().is_empty());
        assert!(matches!(
            exchange(
                &HelperRequest::DisableTun {
                    device: Some("en0".into()),
                },
                uid,
                &fixture.tun,
            ),
            HelperResponse::Error { code, .. } if code == "invalid_tun_config"
        ));
        assert!(fixture.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn backend_failures_map_to_structured_error_responses() {
        let fixture = fake_backend(true);
        *fixture.failure.lock().unwrap() = Some(HelperFailure::new(
            "tun_create_failed",
            "operation not permitted",
        ));
        assert!(fixture.calls.lock().unwrap().is_empty());
        assert!(matches!(
            exchange(
                &HelperRequest::EnableTun {
                    config: TunConfig::verge_default(),
                },
                current_uid(),
                &fixture.tun,
            ),
            HelperResponse::Error { code, message }
                if code == "tun_create_failed" && message == "operation not permitted"
        ));
    }

    #[test]
    fn oversized_requests_are_rejected() {
        let (mut client, server) = UnixStream::pair().unwrap();
        let fixture = fake_backend(true);
        let uid = current_uid();
        let tun = fixture.tun.clone();
        let handle = std::thread::spawn(move || serve_connection(server, uid, &tun).unwrap());
        let oversized = format!(
            "{{\"type\":\"enable_tun\",\"config\":{{\"device\":null,\"address\":\"{}\",\"netmask\":\"255.255.0.0\",\"mtu\":9000,\"routes\":[]}}}}",
            "1".repeat(usize::try_from(MAX_REQUEST_BYTES).unwrap())
        );
        client.write_all(oversized.as_bytes()).unwrap();
        client.write_all(b"\n").unwrap();
        let mut response = String::new();
        BufReader::new(client).read_line(&mut response).unwrap();
        handle.join().unwrap();
        assert!(matches!(
            serde_json::from_str::<HelperResponse>(&response).unwrap(),
            HelperResponse::Error { code, .. } if code == "invalid_request"
        ));
    }

    #[test]
    fn mac_tun_backend_reports_support_only_as_root() {
        let backend = MacTun::default();
        assert_eq!(backend.tun_lifecycle_supported(), current_uid() == 0);
    }

    #[test]
    fn mac_tun_disable_rejects_devices_it_does_not_manage() {
        let mut backend = MacTun::default();
        assert_eq!(
            backend.disable_tun(Some("utun9")).unwrap_err().code,
            "tun_not_managed"
        );
        backend.disable_tun(None).unwrap();
    }
}
