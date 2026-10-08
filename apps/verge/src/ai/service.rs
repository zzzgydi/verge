use super::{
    settings::{self, Credentials, Keychain},
    tools::ToolContext,
    *,
};
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
    channel: crate::identity::AppChannel,
}

impl AiService {
    pub fn new(directory: PathBuf) -> Self {
        Self::for_channel(directory, crate::identity::AppChannel::Stable)
    }
    pub fn for_channel(directory: PathBuf, channel: crate::identity::AppChannel) -> Self {
        Self {
            channel,
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
            return Ok(self.snapshot());
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
        let credentials = Keychain(self.channel);
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
                        saved = settings::save(
                            &directory,
                            &saved,
                            config,
                            api_key,
                            clear_key,
                            &credentials,
                        )?;
                        let mut state = shared.lock().unwrap();
                        state.config = saved.config;
                        state.has_key = saved.credential.is_some();
                        state.activity = "Settings saved".into();
                        return Ok(());
                    }
                    {
                        let mut state = shared.lock().unwrap();
                        state.config = saved.config.clone();
                        state.has_key = saved.credential.is_some();
                        state.revision += 1;
                    }
                    if matches!(command, AiCommand::GetState) {
                        shared.lock().unwrap().activity.clear();
                        return Ok(());
                    }
                    let config = saved.config.validated()?;
                    let key = match saved.credential {
                        Some(account) => credentials.get(&account)?,
                        None => Secret::default(),
                    };
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
                    let session = context.map(|context| {
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
                proposals.lock().unwrap().clear();
                for p in &mut state.proposals {
                    if p.status == "pending" {
                        p.status = "expired".into();
                    }
                }
                state.error = Some(error.message);
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
