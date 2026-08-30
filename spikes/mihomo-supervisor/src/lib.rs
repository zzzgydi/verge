use std::{
    collections::VecDeque,
    io,
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SupervisorConfig {
    pub binary: PathBuf,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
    pub max_restarts: u32,
    pub restart_backoff: Duration,
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

pub struct CoreSupervisor {
    config: SupervisorConfig,
    child: Option<Child>,
    state: SupervisorState,
    restart_attempts: u32,
    events: VecDeque<SupervisorEvent>,
}

impl CoreSupervisor {
    pub fn new(config: SupervisorConfig) -> io::Result<Self> {
        if !config.binary.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("core binary does not exist: {}", config.binary.display()),
            ));
        }
        if !config.working_dir.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "core working directory does not exist: {}",
                    config.working_dir.display()
                ),
            ));
        }

        Ok(Self {
            config,
            child: None,
            state: SupervisorState::Stopped,
            restart_attempts: 0,
            events: VecDeque::new(),
        })
    }

    pub fn state(&self) -> &SupervisorState {
        &self.state
    }

    pub fn start(&mut self) -> io::Result<()> {
        if self.child.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "core process is already running",
            ));
        }

        let child = Command::new(&self.config.binary)
            .args(&self.config.args)
            .current_dir(&self.config.working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let pid = child.id();
        self.child = Some(child);
        self.state = SupervisorState::Running { pid };
        self.events.push_back(SupervisorEvent::Started { pid });
        Ok(())
    }

    pub fn poll(&mut self, now: Instant) -> io::Result<()> {
        if let SupervisorState::Backoff { retry_at, .. } = self.state
            && now >= retry_at
        {
            self.start()?;
            return Ok(());
        }

        let Some(child) = self.child.as_mut() else {
            return Ok(());
        };
        let Some(status) = child.try_wait()? else {
            return Ok(());
        };

        self.child = None;
        self.handle_exit(status, now);
        Ok(())
    }

    pub fn stop(&mut self) -> io::Result<()> {
        if let Some(mut child) = self.child.take()
            && child.try_wait()?.is_none()
        {
            child.kill()?;
            child.wait()?;
        }
        self.restart_attempts = 0;
        self.state = SupervisorState::Stopped;
        self.events.push_back(SupervisorEvent::Stopped);
        Ok(())
    }

    pub fn drain_events(&mut self) -> impl Iterator<Item = SupervisorEvent> + '_ {
        self.events.drain(..)
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
        let retry_at = now + delay;
        self.state = SupervisorState::Backoff {
            attempt: self.restart_attempts,
            retry_at,
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

#[cfg(test)]
mod tests {
    use std::{env, thread};

    use super::*;

    fn config(script: &str, max_restarts: u32) -> SupervisorConfig {
        SupervisorConfig {
            binary: PathBuf::from("/bin/sh"),
            args: vec!["-c".into(), script.into()],
            working_dir: env::temp_dir(),
            max_restarts,
            restart_backoff: Duration::from_millis(1),
        }
    }

    fn wait_for_exit(supervisor: &mut CoreSupervisor) {
        for _ in 0..100 {
            supervisor.poll(Instant::now()).unwrap();
            if !matches!(supervisor.state(), SupervisorState::Running { .. }) {
                return;
            }
            thread::sleep(Duration::from_millis(2));
        }
        panic!("child did not exit in time");
    }

    #[test]
    fn rejects_missing_binary() {
        let mut value = config("exit 0", 0);
        value.binary = PathBuf::from("/missing/verge-mihomo");
        let error = match CoreSupervisor::new(value) {
            Ok(_) => panic!("missing core binary was accepted"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn schedules_bounded_exponential_restarts() {
        let mut supervisor = CoreSupervisor::new(config("exit 7", 2)).unwrap();
        supervisor.start().unwrap();

        wait_for_exit(&mut supervisor);
        assert!(matches!(
            supervisor.state(),
            SupervisorState::Backoff { attempt: 1, .. }
        ));
        let retry_at = match supervisor.state() {
            SupervisorState::Backoff { retry_at, .. } => *retry_at,
            _ => unreachable!(),
        };
        supervisor.poll(retry_at).unwrap();

        wait_for_exit(&mut supervisor);
        assert!(matches!(
            supervisor.state(),
            SupervisorState::Backoff { attempt: 2, .. }
        ));
        let retry_at = match supervisor.state() {
            SupervisorState::Backoff { retry_at, .. } => *retry_at,
            _ => unreachable!(),
        };
        supervisor.poll(retry_at).unwrap();

        wait_for_exit(&mut supervisor);
        assert_eq!(
            supervisor.state(),
            &SupervisorState::Failed {
                attempts: 3,
                last_code: Some(7)
            }
        );
    }

    #[test]
    fn stop_terminates_a_running_process_and_cancels_restart() {
        let mut supervisor = CoreSupervisor::new(config("sleep 30", 3)).unwrap();
        supervisor.start().unwrap();
        assert!(matches!(
            supervisor.state(),
            SupervisorState::Running { .. }
        ));

        supervisor.stop().unwrap();
        assert_eq!(supervisor.state(), &SupervisorState::Stopped);
    }
}
