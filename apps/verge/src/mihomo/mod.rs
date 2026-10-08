mod watchdog;
use std::{
    collections::VecDeque,
    io::{self, BufRead, BufReader, Read},
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
pub(crate) use watchdog::run as core_watch_main;

use crate::domain::{AppError, ErrorCode};

mod controller;
mod endpoint;
pub use endpoint::ControllerEndpoint;
mod realtime;
mod sidecar;

pub use crate::domain::{
    ConnectionSnapshot, LogEvent, MemoryEvent, ProxyGroup, RealtimeEvent, RealtimeTopic, RunMode,
    TrafficEvent,
};
pub use controller::{
    ControllerRequest, ControllerResponse, ControllerTransport, MihomoClient,
    TcpControllerTransport,
};
pub use realtime::{RealtimeOptions, RealtimeSubscription};
pub use sidecar::{ArtifactInstall, SidecarManifest, SidecarTarget, install_verified_artifact};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MihomoConfig {
    pub binary: PathBuf,
    pub working_dir: PathBuf,
    pub controller: SocketAddr,
    pub secret: String,
    pub max_restarts: u32,
    pub restart_backoff: Duration,
    pub stop_timeout: Duration,
    pub log_capacity: usize,
}

impl MihomoConfig {
    pub fn validate(&self) -> Result<(), AppError> {
        if !self.binary.is_file() {
            return Err(core_error(format!(
                "Mihomo binary does not exist: {}",
                self.binary.display()
            )));
        }
        if !self.working_dir.is_dir() {
            return Err(core_error(format!(
                "Mihomo working directory does not exist: {}",
                self.working_dir.display()
            )));
        }
        if !self.controller.ip().is_loopback() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Mihomo controller must bind to a loopback address",
            ));
        }
        if self.secret.is_empty() || self.secret.chars().any(char::is_control) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Mihomo controller secret is empty or contains control characters",
            ));
        }
        if self.log_capacity == 0 {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Mihomo log capacity must be greater than zero",
            ));
        }
        Ok(())
    }
}

pub fn validate_config_file(
    binary: &Path,
    working_dir: &Path,
    candidate: &Path,
) -> Result<(), AppError> {
    validate_config_file_with_timeout(binary, working_dir, candidate, Duration::from_secs(10))
}

fn validate_config_file_with_timeout(
    binary: &Path,
    working_dir: &Path,
    candidate: &Path,
    timeout: Duration,
) -> Result<(), AppError> {
    use std::os::unix::process::CommandExt;
    if !candidate.is_file() {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            format!("candidate config does not exist: {}", candidate.display()),
        ));
    }
    let mut child = Command::new(binary)
        .args(["-t", "-d"])
        .arg(working_dir)
        .arg("-f")
        .arg(candidate)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(core_io_error)?;
    let mut stderr = child.stderr.take().expect("piped validation stderr");
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.by_ref().take(64 * 1024).read_to_end(&mut bytes);
        let _ = io::copy(&mut stderr, &mut io::sink());
        bytes
    });
    let deadline = Instant::now() + timeout;
    let observed = loop {
        match watchdog::exited(&child) {
            Ok(true) => break Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => break Err(core_io_error(error)),
            Ok(false) => {}
        }
        if Instant::now() >= deadline {
            break Err(core_error("Mihomo configuration validation timed out"));
        }
        thread::sleep(Duration::from_millis(10));
    };
    // Kill descendants before reaping the leader or joining its output reader.
    let cleanup = watchdog::kill_group(&child).map_err(core_io_error);
    let status = child.wait().map_err(core_io_error);
    let stderr = reader.join().unwrap_or_default();
    observed?;
    cleanup?;
    if status?.success() {
        return Ok(());
    }
    let message = String::from_utf8_lossy(&stderr);
    Err(AppError::new(
        ErrorCode::CoreRejectedConfig,
        format!("Mihomo rejected candidate config: {}", message.trim()),
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoreHealth {
    pub version: String,
}

pub fn probe_health(
    controller: SocketAddr,
    secret: &str,
    timeout: Duration,
) -> Result<CoreHealth, AppError> {
    let transport = TcpControllerTransport::new(controller, secret, timeout)?;
    let version = MihomoClient::new(transport).version()?;
    Ok(CoreHealth { version })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SupervisorState {
    Stopped,
    Running {
        pid: u32,
    },
    Backoff {
        attempt: u32,
        retry_at: Instant,
    },
    Failed {
        attempts: u32,
        last_code: Option<i32>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SupervisorEvent {
    Started {
        pid: u32,
    },
    Exited {
        code: Option<i32>,
    },
    RestartScheduled {
        attempt: u32,
        delay: Duration,
    },
    Stopped,
    Failed {
        attempts: u32,
        last_code: Option<i32>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoreLogLine {
    pub stream: LogStream,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogStream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug)]
struct LogBuffer {
    capacity: usize,
    lines: Arc<Mutex<VecDeque<CoreLogLine>>>,
}

impl LogBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            lines: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
        }
    }

    fn attach<R>(&self, reader: R, stream: LogStream) -> thread::JoinHandle<()>
    where
        R: Read + Send + 'static,
    {
        let lines = Arc::clone(&self.lines);
        let capacity = self.capacity;
        thread::spawn(move || {
            for message in BufReader::new(reader).lines().map_while(Result::ok) {
                let mut buffer = lines.lock().expect("core log buffer mutex poisoned");
                if buffer.len() == capacity {
                    buffer.pop_front();
                }
                buffer.push_back(CoreLogLine { stream, message });
            }
        })
    }

    fn snapshot(&self) -> Vec<CoreLogLine> {
        self.lines
            .lock()
            .expect("core log buffer mutex poisoned")
            .iter()
            .cloned()
            .collect()
    }
}

pub struct CoreSupervisor {
    config: MihomoConfig,
    config_path: PathBuf,
    internal_socket: Option<PathBuf>,
    child: Option<Child>,
    tun_fd: Option<std::os::fd::OwnedFd>,
    lifeline: Option<std::os::unix::net::UnixStream>,
    leased_child: bool,
    state: SupervisorState,
    restart_attempts: u32,
    events: VecDeque<SupervisorEvent>,
    logs: LogBuffer,
    log_readers: Vec<thread::JoinHandle<()>>,
}

impl CoreSupervisor {
    pub fn new(config: MihomoConfig, config_path: PathBuf) -> Result<Self, AppError> {
        config.validate()?;
        if !config_path.is_file() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                format!("Mihomo config does not exist: {}", config_path.display()),
            ));
        }
        let logs = LogBuffer::new(config.log_capacity);
        Ok(Self {
            config,
            config_path,
            internal_socket: None,
            child: None,
            tun_fd: None,
            lifeline: None,
            leased_child: false,
            state: SupervisorState::Stopped,
            restart_attempts: 0,
            events: VecDeque::new(),
            logs,
            log_readers: Vec::new(),
        })
    }

    pub fn set_internal_socket(&mut self, path: PathBuf) {
        self.internal_socket = Some(path);
    }

    pub(crate) fn set_tun_fd(&mut self, fd: Option<std::os::fd::OwnedFd>) {
        self.tun_fd = fd;
    }

    pub fn state(&self) -> &SupervisorState {
        &self.state
    }

    pub fn logs(&self) -> Vec<CoreLogLine> {
        self.logs.snapshot()
    }

    pub fn validate_candidate(&self, candidate: &Path) -> Result<(), AppError> {
        validate_config_file(&self.config.binary, &self.config.working_dir, candidate)
    }

    pub fn apply_config(&mut self, config_path: &Path) -> Result<(), AppError> {
        if !config_path.is_file() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                format!("Mihomo config does not exist: {}", config_path.display()),
            ));
        }
        self.stop()?;
        self.config_path = config_path.to_owned();
        self.start()
    }

    pub fn health(&self, timeout: Duration) -> Result<CoreHealth, AppError> {
        #[cfg(unix)]
        if let Some(path) = &self.internal_socket {
            let transport = TcpControllerTransport::unix(path.clone(), timeout)?;
            let health = CoreHealth {
                version: MihomoClient::new(transport).version()?,
            };
            // The Unix listener can be healthy even when the external TCP port failed to bind.
            let yaml = std::fs::read_to_string(&self.config_path).map_err(core_io_error)?;
            let value: serde_yaml::Value =
                serde_yaml::from_str(&yaml).map_err(|_| core_error("invalid runtime config"))?;
            if let Some(address) = value
                .get("external-controller")
                .and_then(serde_yaml::Value::as_str)
                .filter(|s| !s.is_empty())
            {
                let mut address: SocketAddr = address
                    .parse()
                    .map_err(|_| core_error("invalid external controller address"))?;
                if address.ip().is_unspecified() {
                    address.set_ip(if address.is_ipv4() {
                        std::net::Ipv4Addr::LOCALHOST.into()
                    } else {
                        std::net::Ipv6Addr::LOCALHOST.into()
                    });
                }
                let secret = value
                    .get("secret")
                    .and_then(serde_yaml::Value::as_str)
                    .unwrap_or_default();
                let endpoint = ControllerEndpoint::Tcp(address);
                let mut stream = endpoint.connect(timeout).map_err(core_io_error)?;
                use std::io::Write as _;
                write!(stream, "GET /version HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {secret}\r\nConnection: close\r\n\r\n").map_err(core_io_error)?;
                let mut response = String::new();
                stream
                    .read_to_string(&mut response)
                    .map_err(core_io_error)?;
                if !response.starts_with("HTTP/1.1 200 ") && !response.starts_with("HTTP/1.0 200 ")
                {
                    return Err(core_error("external controller health check failed"));
                }
            }
            if let Some(device) = value
                .get("tun")
                .filter(|tun| tun["enable"].as_bool() == Some(true))
                .and_then(|tun| tun.get("device"))
                .and_then(serde_yaml::Value::as_str)
            {
                let transport = TcpControllerTransport::unix(path.clone(), timeout)?;
                MihomoClient::new(transport).verify_tun(device)?;
            }
            return Ok(health);
        }
        probe_health(self.config.controller, &self.config.secret, timeout)
    }

    pub fn start(&mut self) -> Result<(), AppError> {
        if self.child.is_some() {
            return Err(AppError::new(
                ErrorCode::Conflict,
                "Mihomo is already running",
            ));
        }
        let mut command = if let Some(fd) = &self.tun_fd {
            let (command, lifeline) =
                watchdog::command(&self.config.binary, fd).map_err(core_io_error)?;
            self.lifeline = Some(lifeline);
            command
        } else {
            Command::new(&self.config.binary)
        };
        let mut child = command
            .arg("-d")
            .arg(&self.config.working_dir)
            .arg("-f")
            .arg(&self.config_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(core_io_error)?;
        if let Some(stdout) = child.stdout.take() {
            self.log_readers
                .push(self.logs.attach(stdout, LogStream::Stdout));
        }
        if let Some(stderr) = child.stderr.take() {
            self.log_readers
                .push(self.logs.attach(stderr, LogStream::Stderr));
        }
        let pid = child.id();
        self.leased_child = self.tun_fd.is_some();
        self.child = Some(child);
        self.state = SupervisorState::Running { pid };
        self.events.push_back(SupervisorEvent::Started { pid });
        Ok(())
    }

    pub fn poll(&mut self, now: Instant) -> Result<(), AppError> {
        if let SupervisorState::Backoff { retry_at, .. } = self.state
            && now >= retry_at
        {
            self.start()?;
            return Ok(());
        }
        let Some(child) = self.child.as_mut() else {
            return Ok(());
        };
        if self.leased_child {
            if !watchdog::exited(child).map_err(core_io_error)? {
                return Ok(());
            }
            watchdog::kill_group(child).map_err(core_io_error)?;
        }
        let Some(status) = child.try_wait().map_err(core_io_error)? else {
            return Ok(());
        };
        self.child = None;
        self.leased_child = false;
        self.lifeline = None;
        self.finish_log_readers();
        self.handle_exit(status, now);
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), AppError> {
        self.lifeline = None;
        if self.leased_child {
            let result = self
                .child
                .as_mut()
                .map(|child| watchdog::stop_group(child, self.config.stop_timeout))
                .transpose()
                .map_err(core_io_error);
            result?;
            self.child = None;
            self.leased_child = false;
            self.finish_stop();
            return Ok(());
        }
        if let Some(mut child) = self.child.take()
            && child.try_wait().map_err(core_io_error)?.is_none()
        {
            if let Err(error) = request_termination(child.id()) {
                let _ = child.kill();
                let _ = child.wait();
                self.finish_stop();
                return Err(error);
            }
            let deadline = Instant::now() + self.config.stop_timeout;
            while Instant::now() < deadline {
                if child.try_wait().map_err(core_io_error)?.is_some() {
                    self.finish_stop();
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(10));
            }
            child.kill().map_err(core_io_error)?;
            child.wait().map_err(core_io_error)?;
        }
        self.finish_stop();
        Ok(())
    }

    pub fn drain_events(&mut self) -> impl Iterator<Item = SupervisorEvent> + '_ {
        self.events.drain(..)
    }

    fn finish_stop(&mut self) {
        self.lifeline = None;
        self.finish_log_readers();
        self.restart_attempts = 0;
        self.state = SupervisorState::Stopped;
        self.events.push_back(SupervisorEvent::Stopped);
    }

    fn finish_log_readers(&mut self) {
        for reader in self.log_readers.drain(..) {
            let _ = reader.join();
        }
    }

    fn handle_exit(&mut self, status: ExitStatus, now: Instant) {
        let code = status.code();
        self.events.push_back(SupervisorEvent::Exited { code });
        self.restart_attempts += 1;
        if self.restart_attempts > self.config.max_restarts {
            self.state = SupervisorState::Failed {
                attempts: self.restart_attempts,
                last_code: code,
            };
            self.events.push_back(SupervisorEvent::Failed {
                attempts: self.restart_attempts,
                last_code: code,
            });
            return;
        }
        let multiplier = 1_u32 << self.restart_attempts.saturating_sub(1).min(8);
        let delay = self.config.restart_backoff.saturating_mul(multiplier);
        self.state = SupervisorState::Backoff {
            attempt: self.restart_attempts,
            retry_at: now + delay,
        };
        self.events.push_back(SupervisorEvent::RestartScheduled {
            attempt: self.restart_attempts,
            delay,
        });
    }
}

impl Drop for CoreSupervisor {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(unix)]
fn request_termination(pid: u32) -> Result<(), AppError> {
    let status = Command::new("/bin/kill")
        .args(["-TERM", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(core_io_error)?;
    if status.success() {
        Ok(())
    } else {
        Err(core_error("failed to send SIGTERM to Mihomo"))
    }
}

#[cfg(not(unix))]
fn request_termination(_pid: u32) -> Result<(), AppError> {
    Err(core_error(
        "graceful Mihomo termination is not implemented on this platform",
    ))
}

fn core_io_error(error: io::Error) -> AppError {
    core_error(error.to_string())
}

fn core_error(message: impl Into<String>) -> AppError {
    AppError::new(ErrorCode::CoreUnavailable, message)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::Write,
        net::TcpListener,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("verge-core-{name}-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn config(directory: &Path) -> MihomoConfig {
        MihomoConfig {
            binary: PathBuf::from("/bin/sh"),
            working_dir: directory.to_owned(),
            controller: "127.0.0.1:9090".parse().unwrap(),
            secret: "test-secret".into(),
            max_restarts: 0,
            restart_backoff: Duration::from_millis(1),
            stop_timeout: Duration::from_millis(200),
            log_capacity: 2,
        }
    }

    #[test]
    fn validation_uses_arguments_without_a_shell() {
        let directory = TestDir::new("validation");
        let candidate = directory.0.join("profile with spaces.yaml");
        fs::write(&candidate, "mode: rule\n").unwrap();
        validate_config_file(Path::new("/usr/bin/true"), &directory.0, &candidate).unwrap();

        let error = validate_config_file(Path::new("/usr/bin/false"), &directory.0, &candidate)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::CoreRejectedConfig);
    }

    #[test]
    fn validation_timeout_reaps_child_and_closes_inherited_output() {
        use std::os::unix::fs::PermissionsExt;
        let directory = TestDir::new("validation-timeout");
        let candidate = directory.0.join("config.yaml");
        fs::write(&candidate, "mode: rule\n").unwrap();
        let script = directory.0.join("validator.sh");
        let pid_file = directory.0.join("pid");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' $$ > '{}'\n/bin/sleep 60 &\nwait\n",
                pid_file.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let start = Instant::now();
        let error = validate_config_file_with_timeout(
            &script,
            &directory.0,
            &candidate,
            Duration::from_secs(2),
        )
        .unwrap_err();
        assert!(error.message.contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(5));
        let pid: i32 = fs::read_to_string(pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    }

    #[test]
    fn supervisor_recovers_when_leased_watchdog_is_killed_or_stuck() {
        use std::os::{
            fd::AsRawFd,
            unix::{net::UnixStream, process::CommandExt},
        };
        for crash in [true, false] {
            let directory = TestDir::new("watchdog-failure");
            let candidate = directory.0.join("config.yaml");
            fs::write(&candidate, "mode: rule\n").unwrap();
            let ready = directory.0.join("ready");
            let (source, mut peer) = UnixStream::pair().unwrap();
            peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut command = Command::new("/bin/sh");
            command
                .args([
                    "-c",
                    "/bin/sleep 60 & printf ready > \"$1\"; wait",
                    "watchdog",
                ])
                .arg(&ready)
                .process_group(0)
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            unsafe {
                command.pre_exec(move || {
                    if libc::dup2(source.as_raw_fd(), 3) < 0 || libc::fcntl(3, libc::F_SETFD, 0) < 0
                    {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let mut child = command.spawn().unwrap();
            drop(command);
            let mut supervisor = CoreSupervisor::new(config(&directory.0), candidate).unwrap();
            supervisor.log_readers.push(
                supervisor
                    .logs
                    .attach(child.stdout.take().unwrap(), LogStream::Stdout),
            );
            supervisor.leased_child = true;
            supervisor.state = SupervisorState::Running { pid: child.id() };
            supervisor.child = Some(child);
            let deadline = Instant::now() + Duration::from_secs(3);
            while !ready.exists() {
                assert!(Instant::now() < deadline);
                thread::sleep(Duration::from_millis(10));
            }
            let pid = supervisor.child.as_ref().unwrap().id() as i32;
            assert_eq!(
                unsafe { libc::kill(pid, if crash { libc::SIGKILL } else { libc::SIGSTOP }) },
                0
            );
            let start = Instant::now();
            if crash {
                while matches!(supervisor.state(), SupervisorState::Running { .. }) {
                    supervisor.poll(Instant::now()).unwrap();
                    assert!(start.elapsed() < Duration::from_secs(3));
                    thread::sleep(Duration::from_millis(10));
                }
            } else {
                supervisor.stop().unwrap();
            }
            assert_eq!(peer.read(&mut [0]).unwrap(), 0, "orphan retained device");
            assert!(start.elapsed() < Duration::from_secs(3));
            assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        }
    }

    #[test]
    fn config_rejects_non_loopback_controller_and_header_injection() {
        let directory = TestDir::new("unsafe-controller");
        let mut settings = config(&directory.0);
        settings.controller = "192.0.2.1:9090".parse().unwrap();
        assert_eq!(
            settings.validate().unwrap_err().code,
            ErrorCode::InvalidInput
        );

        settings.controller = "127.0.0.1:9090".parse().unwrap();
        settings.secret = "secret\r\nInjected: true".into();
        assert_eq!(
            settings.validate().unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }

    #[test]
    fn health_probe_requires_loopback_and_parses_version() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 128];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let size = stream.read(&mut chunk).unwrap();
                if size == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..size]);
            }
            let request = String::from_utf8_lossy(&request);
            assert!(request.contains("Authorization: Bearer test-secret"));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 21\r\nConnection: close\r\n\r\n{\"version\":\"1.2.3\"}",
                )
                .unwrap();
        });

        let health = probe_health(address, "test-secret", Duration::from_secs(1)).unwrap();
        server.join().unwrap();
        assert_eq!(health.version, "1.2.3");
    }

    #[test]
    fn supervisor_captures_bounded_logs_and_detects_exit() {
        let directory = TestDir::new("logs");
        let candidate = directory.0.join("config.yaml");
        fs::write(&candidate, "mode: rule\n").unwrap();
        let mut supervisor = CoreSupervisor::new(config(&directory.0), candidate).unwrap();
        supervisor.config.binary = PathBuf::from("/bin/sh");
        supervisor.config_path = PathBuf::from("-c");
        supervisor.config.working_dir = directory.0.clone();

        let mut child = Command::new("/bin/sh")
            .args(["-c", "printf 'one\\ntwo\\nbad\\n' >&2"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        supervisor.log_readers.push(
            supervisor
                .logs
                .attach(child.stdout.take().unwrap(), LogStream::Stdout),
        );
        supervisor.log_readers.push(
            supervisor
                .logs
                .attach(child.stderr.take().unwrap(), LogStream::Stderr),
        );
        child.wait().unwrap();
        supervisor.finish_log_readers();

        let logs = supervisor.logs();
        assert_eq!(logs.len(), 2);
        assert!(logs.iter().any(|line| line.message == "bad"));
    }

    #[test]
    fn stop_sends_term_before_forced_kill() {
        let directory = TestDir::new("stop");
        let candidate = directory.0.join("-d");
        fs::write(&candidate, "mode: rule\n").unwrap();
        let script = directory.0.join("runner.sh");
        fs::write(
            &script,
            "#!/bin/sh\ntrap 'exit 0' TERM\nwhile true; do sleep 1; done\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&script).unwrap().permissions();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            permissions.set_mode(0o755);
        }
        fs::set_permissions(&script, permissions).unwrap();

        let mut settings = config(&directory.0);
        settings.binary = script;
        let mut supervisor = CoreSupervisor::new(settings, candidate).unwrap();
        supervisor.start().unwrap();
        supervisor.stop().unwrap();
        assert_eq!(supervisor.state(), &SupervisorState::Stopped);
    }
}
