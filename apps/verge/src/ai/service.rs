use super::{settings, tools::ToolContext, *};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

/// Worker writes one coalesced snapshot. The daemon publishes at most once per tick.
pub struct AiService {
    state: Arc<Mutex<AiSnapshot>>,
    cancelled: Arc<AtomicBool>,
    directory: PathBuf,
    loaded: bool,
    proposals: Arc<Mutex<proposals::Proposals>>,
}

impl AiService {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            state: Arc::new(Mutex::new(AiSnapshot::default())),
            cancelled: Arc::new(AtomicBool::new(false)),
            directory,
            loaded: false,
            proposals: Arc::new(Mutex::new(proposals::Proposals::default())),
        }
    }
    pub(crate) fn take_proposal(
        &mut self,
        id: &str,
        digest: &str,
    ) -> Result<proposals::Pending, AppError> {
        if self.snapshot().busy {
            return Err(error("Wait for the AI operation to finish"));
        }
        self.proposals.lock().unwrap().consume(id, digest)
    }
    #[cfg(test)]
    pub(crate) fn proposal_store(&self) -> Arc<Mutex<proposals::Proposals>> {
        self.proposals.clone()
    }
    pub(crate) fn finish_proposal(&mut self, id: &str, status: &str, result: &str) -> AiSnapshot {
        let mut state = self.state.lock().unwrap();
        if let Some(proposal) = state.proposals.iter_mut().find(|p| p.id == id)
            && proposal.status == "pending"
        {
            proposal.status = status.into();
            proposal.result = Some(result.into());
        }
        state.revision += 1;
        state.clone()
    }
    pub fn snapshot(&self) -> AiSnapshot {
        self.state.lock().unwrap().clone()
    }
    pub fn revision(&self) -> u64 {
        self.state.lock().unwrap().revision
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn handle(
        &mut self,
        command: AiCommand,
        context: Option<ToolContext>,
    ) -> Result<AiSnapshot, AppError> {
        if let AiCommand::Dismiss { id } = &command {
            self.proposals.lock().unwrap().dismiss(id);
            return Ok(self.finish_proposal(id, "dismissed", "No changes applied"));
        }
        if matches!(command, AiCommand::Approve { .. }) {
            return Err(error("Confirmation must be handled by the application"));
        }
        if matches!(command, AiCommand::Cancel) {
            self.cancel();
            return Ok(self.snapshot());
        }
        if matches!(command, AiCommand::GetState) && self.loaded {
            let state = self.snapshot();
            if state.busy || state.operation != Some(AiOperation::State) || state.error.is_none() {
                return Ok(state);
            }
        }
        {
            let mut state = self.state.lock().unwrap();
            if state.busy {
                return Err(error("An AI operation is already running"));
            }
            if matches!(command, AiCommand::Clear) {
                self.proposals.lock().unwrap().clear();
                state.proposals.clear();
                state.messages.clear();
                state.evidence.clear();
                state.error = None;
                state.activity.clear();
                state.revision += 1;
                return Ok(state.clone());
            }
            if matches!(command, AiCommand::Retry) {
                retry_turn(&mut state)?;
            }
            if let AiCommand::Start { prompt } = &command {
                if prompt.trim().is_empty() || prompt.len() > 8192 {
                    return Err(error("Enter a question of at most 8 KiB"));
                }
                if state.messages.len() >= 16
                    || state.messages.iter().map(|m| m.text.len()).sum::<usize>() > 96 * 1024
                {
                    return Err(error(
                        "Conversation limit reached; clear it to start a new conversation",
                    ));
                }
                state.messages.push(ChatMessage {
                    role: "user".into(),
                    text: prompt.trim().into(),
                    evidence: Vec::new(),
                });
            }
            if matches!(
                command,
                AiCommand::Start { .. } | AiCommand::Retry | AiCommand::SaveConfig { .. }
            ) {
                state.evidence.clear();
                self.proposals.lock().unwrap().clear();
                for p in &mut state.proposals {
                    if p.status == "pending" {
                        p.status = "expired".into();
                    }
                }
                let excess = state.proposals.len().saturating_sub(24);
                state.proposals.drain(..excess);
            }
            state.operation = Some(command.operation());
            state.busy = true;
            state.error = None;
            state.activity = "Preparing".into();
            state.run_id += 1;
            state.revision += 1;
        }
        self.loaded = true;
        self.cancelled = Arc::new(AtomicBool::new(false));
        let cancel = self.cancelled.clone();
        let shared = self.state.clone();
        let directory = self.directory.clone();
        let proposals = self.proposals.clone();
        std::thread::spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || -> Result<(), AppError> {
                    let mut saved = settings::load(&directory)?;
                    if let AiCommand::SaveConfig {
                        config,
                        api_key,
                        clear_key,
                    } = command
                    {
                        saved = settings::save(&directory, &saved, config, api_key, clear_key)?;
                        let mut state = shared.lock().unwrap();
                        state.has_key = saved.has_key();
                        state.config = saved.config;
                        state.activity = "Settings saved".into();
                        return Ok(());
                    }
                    {
                        let mut state = shared.lock().unwrap();
                        state.config = saved.config.clone();
                        state.has_key = saved.has_key();
                        state.revision += 1;
                    }
                    if matches!(command, AiCommand::GetState) {
                        shared.lock().unwrap().activity.clear();
                        return Ok(());
                    }
                    let config = saved.config.validated()?;
                    let key = saved.api_key;
                    if cancel.load(Ordering::Relaxed) {
                        return Err(error("Cancelled"));
                    }
                    let test = matches!(command, AiCommand::TestProvider);
                    let mut evidence = if test {
                        Vec::new()
                    } else {
                        tools::collect(
                            context
                                .clone()
                                .ok_or_else(|| error("Diagnostic context unavailable"))?,
                            &cancel,
                        )
                    };
                    if cancel.load(Ordering::Relaxed) {
                        return Err(error("Cancelled"));
                    }
                    let history = {
                        let mut state = shared.lock().unwrap();
                        for item in &mut evidence {
                            item.id = format!("R{}-{}", state.run_id, item.id);
                        }
                        if !test {
                            state.evidence = evidence.clone();
                        }
                        let history = state
                            .messages
                            .iter()
                            .filter(|m| !m.text.is_empty())
                            .cloned()
                            .collect();
                        if !test {
                            state.messages.push(ChatMessage {
                                role: "assistant".into(),
                                text: String::new(),
                                evidence: evidence.clone(),
                            });
                        }
                        state.revision += 1;
                        history
                    };
                    let session = context.filter(|_| !test).map(|context| {
                        Arc::new(Mutex::new(diagnostics::Session::new(
                            context,
                            evidence.clone(),
                            shared.lock().unwrap().run_id,
                            proposals.clone(),
                            cancel.clone(),
                        )))
                    });
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|_| error("Unable to start AI worker"))?;
                    let result = runtime.block_on(provider::run_with_session(
                        config,
                        key,
                        history,
                        evidence,
                        test,
                        cancel.clone(),
                        |text, activity| {
                            let mut state = shared.lock().unwrap();
                            if let Some(session) = &session {
                                let session = session.lock().unwrap();
                                state.evidence = session.evidence.clone();
                                if let Some(message) = state.messages.last_mut() {
                                    message.evidence = session.evidence.clone();
                                }
                                for proposal in &session.views {
                                    if !state.proposals.iter().any(|p| p.id == proposal.id) {
                                        state.proposals.push(proposal.clone());
                                    }
                                }
                            }
                            state.activity = activity;
                            if !test
                                && !text.is_empty()
                                && let Some(message) = state.messages.last_mut()
                            {
                                message.text = text;
                            }
                            state.revision += 1;
                        },
                        session.clone(),
                    ))?;
                    let mut state = shared.lock().unwrap();
                    if test {
                        state.activity = "Provider connection succeeded".into();
                    } else {
                        if let Some(message) = state.messages.last_mut() {
                            message.text = result;
                        }
                        state.activity = "Completed".into();
                    }
                    Ok(())
                },
            ));
            let result = outcome
                .unwrap_or_else(|_| Err(error("AI worker failed; proxy operation is unaffected")));
            let mut state = shared.lock().unwrap_or_else(|p| p.into_inner());
            if let Err(error) = result {
                let was_cancelled = error.message == "Cancelled";
                cancel.store(true, Ordering::Relaxed);
                if matches!(
                    state.operation,
                    Some(AiOperation::Start | AiOperation::Retry)
                ) {
                    proposals.lock().unwrap().clear();
                    for p in &mut state.proposals {
                        if p.status == "pending" {
                            p.status = "expired".into();
                        }
                    }
                }
                state.error = if was_cancelled {
                    None
                } else {
                    Some(error.message)
                };
                state.activity = if was_cancelled { "Cancelled" } else { "Failed" }.into();
            }
            state.busy = false;
            state.revision += 1;
        });
        Ok(self.snapshot())
    }
}

impl Drop for AiService {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Regenerate the current turn without duplicating the question or consuming a turn.
fn retry_turn(state: &mut AiSnapshot) -> Result<(), AppError> {
    let index = state
        .messages
        .iter()
        .rposition(|m| m.role == "user")
        .ok_or_else(|| error("No question to retry"))?;
    state.messages.truncate(index + 1);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wait_idle(service: &AiService) -> AiSnapshot {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let state = service.snapshot();
            if !state.busy {
                return state;
            }
            assert!(std::time::Instant::now() < deadline, "AI worker stalled");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn failed_initial_settings_load_can_be_retried() {
        let directory = crate::config::tests::TestDir::new("ai-load-retry");
        let path = directory.0.join("settings.json");
        std::fs::write(&path, b"invalid json").unwrap();
        let mut service = AiService::new(directory.0.clone());
        service.handle(AiCommand::GetState, None).unwrap();
        assert!(wait_idle(&service).error.is_some());
        std::fs::remove_file(path).unwrap();
        service.handle(AiCommand::GetState, None).unwrap();
        assert!(wait_idle(&service).error.is_none());
    }

    #[test]
    fn stopping_provider_test_is_not_an_error_and_preserves_the_chat() {
        use std::io::Read;
        use std::net::TcpListener;
        use std::time::Duration;
        let directory = crate::config::tests::TestDir::new("ai-stop-test");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let config = ProviderConfig {
            base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
            model: "fake".into(),
            timeout_seconds: 5,
            ..Default::default()
        };
        settings::save(
            &directory.0,
            &settings::SavedConfig::default(),
            config,
            Secret::default(),
            false,
        )
        .unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = [0; 4096];
            assert!(socket.read(&mut bytes).unwrap() > 0);
            tx.send(()).unwrap();
            loop {
                match socket.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        });
        let mut service = AiService::new(directory.0.clone());
        let evidence = vec![tools::Evidence {
            id: "R1-E1".into(),
            source: "runtime_status".into(),
            captured_at: 1,
            data: serde_json::json!({"mode":"Rule"}),
        }];
        let proposal = proposals::Proposal {
            id: "existing-proposal".into(),
            run_id: 1,
            digest: "bound-digest".into(),
            kind: "mode".into(),
            target: "Direct".into(),
            expires_at: proposals::now() + 300,
            status: "pending".into(),
            changes: vec![],
            evidence_id: "R1-E1".into(),
            result: None,
        };
        {
            let mut state = service.state.lock().unwrap();
            state.messages = vec![ChatMessage {
                role: "assistant".into(),
                text: "Existing answer".into(),
                evidence: evidence.clone(),
            }];
            state.evidence = evidence.clone();
            state.proposals = vec![proposal.clone()];
        }
        let context = ToolContext {
            socket: directory.0.join("unused.sock"),
            services: vec![],
            recovery_path: directory.0.join("unused-recovery.json"),
            config_selected: false,
            connections: None,
            errors: Default::default(),
            config: None,
        };
        service
            .handle(AiCommand::TestProvider, Some(context))
            .unwrap();
        rx.recv_timeout(Duration::from_secs(3)).unwrap();
        service.handle(AiCommand::Cancel, None).unwrap();
        let state = wait_idle(&service);
        server.join().unwrap();
        assert!(state.error.is_none());
        assert_eq!(state.activity, "Cancelled");
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].text, "Existing answer");
        assert_eq!(state.messages[0].evidence, evidence);
        assert_eq!(state.evidence, evidence);
        assert_eq!(state.proposals, vec![proposal]);
    }

    #[test]
    fn saved_config_key_is_used_after_restart_without_entering_snapshots() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::{Duration, Instant};
        let directory = crate::config::tests::TestDir::new("ai-service-file-key");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let config = ProviderConfig {
            base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
            model: "local-test".into(),
            ..Default::default()
        };
        let mut service = AiService::new(directory.0.clone());
        service
            .handle(
                AiCommand::SaveConfig {
                    config,
                    api_key: Secret("test-file-key".into()),
                    clear_key: false,
                },
                None,
            )
            .unwrap();
        let saved = wait_idle(&service);
        assert!(saved.error.is_none());
        assert!(saved.has_key);
        drop(service);
        let mut service = AiService::new(directory.0.clone());
        service.handle(AiCommand::GetState, None).unwrap();
        let loaded = wait_idle(&service);
        assert!(loaded.has_key);
        assert!(
            !serde_json::to_string(&loaded)
                .unwrap()
                .contains("test-file-key")
        );
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "Provider was not called");
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                socket.read_exact(&mut byte).unwrap();
                header.push(byte[0]);
            }
            let header = String::from_utf8(header).unwrap().to_ascii_lowercase();
            assert!(header.starts_with("post /v1/chat/completions "));
            assert!(
                header
                    .lines()
                    .any(|line| line == "authorization: bearer test-file-key")
            );
            let length: usize = header
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .unwrap()
                .parse()
                .unwrap();
            let mut body = vec![0; length];
            socket.read_exact(&mut body).unwrap();
            assert!(!String::from_utf8(body).unwrap().contains("test-file-key"));
            let body = "data: {\"choices\":[{\"delta\":{\"content\":\"OK\"},\"finish_reason\":\"stop\"}]}\r\n\r\n";
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        service.handle(AiCommand::TestProvider, None).unwrap();
        let tested = wait_idle(&service);
        server.join().unwrap();
        assert!(tested.error.is_none(), "{:?}", tested.error);
        assert_eq!(tested.activity, "Provider connection succeeded");
        assert!(!format!("{tested:?}").contains("test-file-key"));
    }

    #[test]
    fn retry_preserves_prior_turns_and_one_current_question() {
        let mut state = AiSnapshot::default();
        for (role, text) in [
            ("user", "first"),
            ("assistant", "answer"),
            ("user", "second"),
            ("assistant", "partial"),
        ] {
            state.messages.push(ChatMessage {
                role: role.into(),
                text: text.into(),
                evidence: vec![],
            });
        }
        retry_turn(&mut state).unwrap();
        retry_turn(&mut state).unwrap();
        assert_eq!(state.messages.len(), 3);
        assert_eq!(state.messages[1].text, "answer");
        assert_eq!(state.messages[2].text, "second");
        assert!(retry_turn(&mut AiSnapshot::default()).is_err());
    }
}
