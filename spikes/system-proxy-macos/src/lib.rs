use std::{io, process::Command};

const NETWORK_SETUP: &str = "/usr/sbin/networksetup";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProxyEndpoint {
    pub host: String,
    pub port: u16,
}

impl ProxyEndpoint {
    pub fn new(host: impl Into<String>, port: u16) -> io::Result<Self> {
        let host = host.into();
        if host.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "proxy host is empty",
            ));
        }
        if port == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "proxy port is zero",
            ));
        }
        Ok(Self { host, port })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProxyProtocolState {
    pub enabled: bool,
    pub endpoint: ProxyEndpoint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProxySnapshot {
    pub web: ProxyProtocolState,
    pub secure_web: ProxyProtocolState,
}

pub trait CommandRunner {
    fn run(&mut self, program: &str, args: &[String]) -> io::Result<String>;
}

#[derive(Default)]
pub struct ProcessRunner;

impl CommandRunner for ProcessRunner {
    fn run(&mut self, program: &str, args: &[String]) -> io::Result<String> {
        let output = Command::new(program).args(args).output()?;
        if !output.status.success() {
            return Err(io::Error::other(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        String::from_utf8(output.stdout)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
}

pub struct MacSystemProxy<R> {
    runner: R,
}

impl<R: CommandRunner> MacSystemProxy<R> {
    pub fn new(runner: R) -> Self {
        Self { runner }
    }

    pub fn snapshot(&mut self, service: &str) -> io::Result<ProxySnapshot> {
        validate_service(service)?;
        let web = self.get_protocol("-getwebproxy", service)?;
        let secure_web = self.get_protocol("-getsecurewebproxy", service)?;
        Ok(ProxySnapshot { web, secure_web })
    }

    pub fn enable(&mut self, service: &str, endpoint: &ProxyEndpoint) -> io::Result<ProxySnapshot> {
        validate_service(service)?;
        let previous = self.snapshot(service)?;
        if let Err(error) = self.apply_endpoint(service, endpoint) {
            let rollback = self.restore(service, &previous);
            return match rollback {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(io::Error::other(format!(
                    "apply failed: {error}; rollback failed: {rollback_error}"
                ))),
            };
        }
        Ok(previous)
    }

    pub fn restore(&mut self, service: &str, snapshot: &ProxySnapshot) -> io::Result<()> {
        validate_service(service)?;
        self.set_protocol("web", service, &snapshot.web)?;
        self.set_protocol("secureweb", service, &snapshot.secure_web)
    }

    fn get_protocol(&mut self, action: &str, service: &str) -> io::Result<ProxyProtocolState> {
        let output = self
            .runner
            .run(NETWORK_SETUP, &[action.to_owned(), service.to_owned()])?;
        parse_protocol_state(&output)
    }

    fn apply_endpoint(&mut self, service: &str, endpoint: &ProxyEndpoint) -> io::Result<()> {
        let state = ProxyProtocolState {
            enabled: true,
            endpoint: endpoint.clone(),
        };
        self.set_protocol("web", service, &state)?;
        self.set_protocol("secureweb", service, &state)
    }

    fn set_protocol(
        &mut self,
        protocol: &str,
        service: &str,
        state: &ProxyProtocolState,
    ) -> io::Result<()> {
        let set_action = format!("-set{protocol}proxy");
        self.runner.run(
            NETWORK_SETUP,
            &[
                set_action,
                service.to_owned(),
                state.endpoint.host.clone(),
                state.endpoint.port.to_string(),
            ],
        )?;
        let state_action = format!("-set{protocol}proxystate");
        self.runner.run(
            NETWORK_SETUP,
            &[
                state_action,
                service.to_owned(),
                if state.enabled { "on" } else { "off" }.to_owned(),
            ],
        )?;
        Ok(())
    }
}

fn validate_service(service: &str) -> io::Result<()> {
    if service.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "network service is empty",
        ));
    }
    Ok(())
}

fn parse_protocol_state(output: &str) -> io::Result<ProxyProtocolState> {
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
            "Port" => {
                port = Some(
                    value
                        .trim()
                        .parse::<u16>()
                        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
                )
            }
            _ => {}
        }
    }

    Ok(ProxyProtocolState {
        enabled: enabled.ok_or_else(|| invalid_output("missing Enabled"))?,
        endpoint: ProxyEndpoint::new(
            host.ok_or_else(|| invalid_output("missing Server"))?,
            port.ok_or_else(|| invalid_output("missing Port"))?,
        )?,
    })
}

fn invalid_output(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    #[derive(Default)]
    struct FakeRunner {
        outputs: VecDeque<io::Result<String>>,
        calls: Vec<Vec<String>>,
    }

    impl CommandRunner for FakeRunner {
        fn run(&mut self, program: &str, args: &[String]) -> io::Result<String> {
            let mut call = vec![program.to_owned()];
            call.extend_from_slice(args);
            self.calls.push(call);
            self.outputs
                .pop_front()
                .unwrap_or_else(|| Ok(String::new()))
        }
    }

    fn output(enabled: bool, host: &str, port: u16) -> String {
        format!(
            "Enabled: {}\nServer: {host}\nPort: {port}\nAuthenticated Proxy Enabled: 0\n",
            if enabled { "Yes" } else { "No" }
        )
    }

    #[test]
    fn parses_networksetup_output() {
        assert_eq!(
            parse_protocol_state(&output(true, "127.0.0.1", 7890)).unwrap(),
            ProxyProtocolState {
                enabled: true,
                endpoint: ProxyEndpoint::new("127.0.0.1", 7890).unwrap()
            }
        );
    }

    #[test]
    fn enable_returns_previous_snapshot_and_uses_argument_arrays() {
        let mut runner = FakeRunner::default();
        runner
            .outputs
            .push_back(Ok(output(false, "old.local", 8080)));
        runner
            .outputs
            .push_back(Ok(output(true, "secure.local", 8443)));
        let mut proxy = MacSystemProxy::new(runner);

        let previous = proxy
            .enable("Wi-Fi", &ProxyEndpoint::new("127.0.0.1", 7890).unwrap())
            .unwrap();

        assert!(!previous.web.enabled);
        assert!(previous.secure_web.enabled);
        assert_eq!(proxy.runner.calls.len(), 6);
        assert_eq!(
            proxy.runner.calls[2],
            vec![NETWORK_SETUP, "-setwebproxy", "Wi-Fi", "127.0.0.1", "7890"]
        );
    }

    #[test]
    fn partial_apply_failure_attempts_full_rollback() {
        let mut runner = FakeRunner::default();
        runner
            .outputs
            .push_back(Ok(output(false, "old.local", 8080)));
        runner
            .outputs
            .push_back(Ok(output(false, "old.local", 8080)));
        runner.outputs.push_back(Ok(String::new()));
        runner.outputs.push_back(Ok(String::new()));
        runner
            .outputs
            .push_back(Err(io::Error::other("secure proxy failed")));
        let mut proxy = MacSystemProxy::new(runner);

        let error = proxy
            .enable("Wi-Fi", &ProxyEndpoint::new("127.0.0.1", 7890).unwrap())
            .unwrap_err();

        assert!(error.to_string().contains("secure proxy failed"));
        assert_eq!(proxy.runner.calls.len(), 9);
        assert_eq!(proxy.runner.calls[5][1], "-setwebproxy");
        assert_eq!(proxy.runner.calls[7][1], "-setsecurewebproxy");
    }
}
