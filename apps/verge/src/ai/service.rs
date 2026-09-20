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
        }
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
            if matches!(command, AiCommand::Start { .. } | AiCommand::Retry) {
                state.evidence.clear();
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
                            context.ok_or_else(|| error("Diagnostic context unavailable"))?,
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
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|_| error("Unable to start AI worker"))?;
                    let result = runtime.block_on(provider::run(
                        config,
                        key,
                        history,
                        evidence,
                        test,
                        cancel.clone(),
                        |text, activity| {
                            let mut state = shared.lock().unwrap();
                            state.activity = activity;
                            if !test
                                && !text.is_empty()
                                && let Some(message) = state.messages.last_mut()
                            {
                                message.text = text;
                            }
                            state.revision += 1;
                        },
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
                state.error = Some(error.message);
                state.activity = if cancel.load(Ordering::Relaxed) {
                    "Cancelled"
                } else {
                    "Failed"
                }
                .into();
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
