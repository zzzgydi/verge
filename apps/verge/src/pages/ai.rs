//! Conversation and provider drafts stay separate from daemon snapshots.
mod conversation;
pub(super) mod presentation;
mod proposals;

use super::components::{PageHeader, panel};
use crate::{
    ai::{AiCommand, AiOperation, AiSnapshot, ProviderConfig},
    i18n::{Lang, tr},
    ui::{Page, UiResponse},
    view::MainView,
};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{InputEvent, InputState, Textarea, TextareaState},
    scroll::ScrollableElement as _,
    text::TextView,
    v_flex,
};
use gpui_kit::*;
use presentation::{error_text, status_text};

pub struct AiForm {
    pub base: Entity<InputState>,
    pub model: Entity<InputState>,
    pub key: Entity<InputState>,
    pub timeout: Entity<InputState>,
    pub steps: Entity<InputState>,
    pub prompt: Entity<TextareaState>,
    pub advanced_open: bool,
    pub evidence_open: Option<usize>,
    pub clear_key: bool,
    pub confirm_clear: bool,
    pub confirm_proposal: Option<String>,
    pub local_error: Option<String>,
    pub settings_error: Option<String>,
    pub stop_requested: bool,
    test_after_save: bool,
    pub copied: Option<usize>,
    pub error_details: bool,
    pub data_scope_open: bool,
    pub scroll: ScrollHandle,
    pending_prompt: Option<String>,
    pub(super) save_requested: bool,
    pending_save: Option<u64>,
    applied: Option<ProviderConfig>,
}

impl AiForm {
    pub fn new(window: &mut Window, cx: &mut Context<MainView>) -> Self {
        let lang = Lang::En;
        let prompt = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(tr(lang, "ai.prompt"))
                .submit_on_enter(true)
        });
        cx.subscribe_in(&prompt, window, |this, input, event, window, cx| {
            if this.state.page == Page::Ai
                && matches!(event, InputEvent::PressEnter { shift: false, .. })
            {
                let composing = input.update(cx, |input, cx| {
                    input.marked_text_range(window, cx).is_some()
                });
                if !composing {
                    this.send_ai(window, cx);
                }
            }
            cx.notify();
        })
        .detach();
        let form = Self {
            base: cx
                .new(|cx| InputState::new(window, cx).placeholder("https://api.example.com/v1")),
            model: cx.new(|cx| InputState::new(window, cx).placeholder("Model ID")),
            key: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("API key")
            }),
            timeout: cx.new(|cx| InputState::new(window, cx).default_value("60")),
            steps: cx.new(|cx| InputState::new(window, cx).default_value("4")),
            prompt,
            advanced_open: false,
            evidence_open: None,
            clear_key: false,
            confirm_clear: false,
            confirm_proposal: None,
            local_error: None,
            settings_error: None,
            stop_requested: false,
            test_after_save: false,
            copied: None,
            error_details: false,
            data_scope_open: false,
            scroll: ScrollHandle::new(),
            pending_prompt: None,
            save_requested: false,
            pending_save: None,
            applied: None,
        };
        for field in [
            &form.base,
            &form.model,
            &form.key,
            &form.timeout,
            &form.steps,
        ] {
            cx.subscribe(field, |this, input, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.ai_form.settings_error = None;
                    if input == this.ai_form.key && !input.read(cx).value().is_empty() {
                        this.ai_form.clear_key = false;
                    }
                }
                cx.notify();
            })
            .detach();
        }
        form
    }

    pub fn reset_sync(&mut self) {
        // A reconnect must not replace an unsaved provider draft.
        self.disconnected();
    }
    pub fn disconnected(&mut self) {
        self.confirm_proposal = None;
        self.pending_prompt = None;
        self.pending_save = None;
        self.save_requested = false;
        self.test_after_save = false;
        self.stop_requested = false;
    }
    pub fn sync(&mut self, state: &AiSnapshot, window: &mut Window, cx: &mut Context<MainView>) {
        if self.has_provider_draft(cx)
            || state.busy
            || state.revision == 0
            || self.applied.as_ref() == Some(&state.config)
        {
            return;
        }
        for (field, value) in [
            (&self.base, state.config.base_url.clone()),
            (&self.model, state.config.model.clone()),
            (&self.timeout, state.config.timeout_seconds.to_string()),
            (&self.steps, state.config.max_tool_steps.to_string()),
        ] {
            field.update(cx, |input, cx| input.set_value(value, window, cx));
        }
        self.applied = Some(state.config.clone());
    }
    pub(crate) fn has_provider_draft(&self, cx: &App) -> bool {
        let defaults = ProviderConfig::default();
        let applied = self.applied.as_ref().unwrap_or(&defaults);
        self.base.read(cx).value().as_str() != applied.base_url
            || self.model.read(cx).value().as_str() != applied.model
            || self.timeout.read(cx).value().as_str() != applied.timeout_seconds.to_string()
            || self.steps.read(cx).value().as_str() != applied.max_tool_steps.to_string()
            || !self.key.read(cx).value().is_empty()
            || self.clear_key
    }

    fn at_bottom(&self) -> bool {
        self.scroll.max_offset().y + self.scroll.offset().y < px(48.)
    }
}

impl MainView {
    pub(crate) fn open_ai_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_category = super::settings::SettingsCategory::Ai;
        self.navigate(Page::Settings, cx);
    }

    pub(crate) fn send_ai(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.ai.busy
            || self.is_pending(&["ai_write"])
            || self.state.ai.config.base_url.is_empty()
        {
            return;
        }
        let prompt = self.ai_form.prompt.read(cx).value().to_string();
        if prompt.trim().is_empty() {
            return;
        }
        if prompt.len() > 8192 {
            self.ai_form.local_error = Some(tr(self.lang(), "ai.too_long").into());
            cx.notify();
            return;
        }
        self.ai_form.local_error = None;
        if self.try_dispatch_ai(
            AiCommand::Start {
                prompt: prompt.clone(),
            },
            cx,
        ) {
            self.ai_form.pending_prompt = Some(prompt);
            self.ai_form.evidence_open = None;
            self.ai_form.copied = None;
            self.ai_form.scroll.scroll_to_bottom();
        }
        self.ai_form
            .prompt
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// Only an accepted daemon response consumes a draft; transport rejection never does.
    pub(crate) fn ai_response(
        &mut self,
        response: &UiResponse,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let UiResponse::Ai { operation, result } = response else {
            return;
        };
        match result {
            Ok(snapshot) => {
                if matches!(
                    operation,
                    AiOperation::Save | AiOperation::Test | AiOperation::State
                ) && snapshot.error.is_none()
                {
                    self.ai_form.settings_error = None;
                }
                if snapshot.revision >= self.state.ai.revision && !snapshot.busy {
                    self.ai_form.stop_requested = false;
                }
                if *operation == AiOperation::Clear {
                    self.ai_form.local_error = None;
                    self.ai_form.copied = None;
                    self.ai_form.confirm_clear = false;
                    self.ai_form.confirm_proposal = None;
                    self.ai_form.evidence_open = None;
                }
                if snapshot.revision >= self.state.ai.revision
                    && self.ai_form.at_bottom()
                    && (snapshot.messages != self.state.ai.messages
                        || snapshot.proposals != self.state.ai.proposals
                        || snapshot.error != self.state.ai.error
                        || snapshot.activity != self.state.ai.activity
                        || snapshot.busy != self.state.ai.busy)
                {
                    self.ai_form.scroll.scroll_to_bottom();
                }
                if *operation == AiOperation::Start
                    && let Some(sent) = self.ai_form.pending_prompt.take()
                    && self.ai_form.prompt.read(cx).value().as_str() == sent
                {
                    self.ai_form
                        .prompt
                        .update(cx, |input, cx| input.set_value("", window, cx));
                }
                if *operation == AiOperation::Save && self.ai_form.save_requested {
                    self.ai_form.pending_save = Some(snapshot.run_id);
                }
            }
            Err(error) => {
                let message = error_text(self.lang(), &error.message);
                if matches!(
                    operation,
                    AiOperation::Save | AiOperation::Test | AiOperation::State
                ) {
                    self.ai_form.settings_error = Some(message);
                } else {
                    self.ai_form.local_error = Some(message);
                }
                if *operation == AiOperation::Cancel {
                    self.ai_form.stop_requested = false;
                }
                if *operation == AiOperation::Start {
                    self.ai_form.pending_prompt = None;
                }
                if *operation == AiOperation::Save {
                    self.ai_form.save_requested = false;
                    self.ai_form.pending_save = None;
                }
            }
        }
    }

    pub(crate) fn finish_ai_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ai = &self.state.ai;
        if self.ai_form.pending_save != Some(ai.run_id) || ai.busy {
            return;
        }
        self.ai_form.pending_save = None;
        self.ai_form.save_requested = false;
        if ai.error.is_none() && ai.operation == Some(AiOperation::Save) {
            self.ai_form
                .key
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.ai_form.clear_key = false;
            self.ai_form.applied = None;
            for (field, value) in [
                (&self.ai_form.base, ai.config.base_url.clone()),
                (&self.ai_form.model, ai.config.model.clone()),
                (&self.ai_form.timeout, ai.config.timeout_seconds.to_string()),
                (&self.ai_form.steps, ai.config.max_tool_steps.to_string()),
            ] {
                field.update(cx, |input, cx| input.set_value(value, window, cx));
            }
            self.ai_form.applied = Some(ai.config.clone());
            if self.ai_form.test_after_save {
                self.ai_form.test_after_save = false;
                self.try_dispatch_ai(AiCommand::TestProvider, cx);
            }
        }
    }
}

impl MainView {
    pub(crate) fn stop_ai(&mut self, cx: &mut Context<Self>) {
        if !self.state.ai.busy || self.ai_form.stop_requested {
            return;
        }
        if self.try_dispatch_ai(AiCommand::Cancel, cx) {
            self.ai_form.stop_requested = true;
        }
    }

    pub(crate) fn save_ai_settings(&mut self, test_after_save: bool, cx: &mut Context<Self>) {
        if self.state.ai.busy || self.is_pending(&["ai_write"]) {
            return;
        }
        self.ai_form.settings_error = None;
        self.ai_form.error_details = false;
        if test_after_save
            && !self.ai_form.has_provider_draft(cx)
            && !self.state.ai.config.base_url.is_empty()
        {
            self.try_dispatch_ai(AiCommand::TestProvider, cx);
            return;
        }
        let config = ProviderConfig {
            base_url: self.ai_form.base.read(cx).value().to_string(),
            model: self.ai_form.model.read(cx).value().to_string(),
            timeout_seconds: self
                .ai_form
                .timeout
                .read(cx)
                .value()
                .trim()
                .parse()
                .unwrap_or(0),
            max_tool_steps: self
                .ai_form
                .steps
                .read(cx)
                .value()
                .trim()
                .parse()
                .unwrap_or(0),
        };
        let config = match config.validated() {
            Ok(config) => config,
            Err(error) => {
                self.ai_form.settings_error = Some(error_text(self.lang(), &error.message));
                cx.notify();
                return;
            }
        };
        if self.try_dispatch_ai(
            AiCommand::SaveConfig {
                config,
                api_key: crate::ai::Secret(self.ai_form.key.read(cx).value().trim().to_string()),
                clear_key: self.ai_form.clear_key,
            },
            cx,
        ) {
            self.ai_form.save_requested = true;
            self.ai_form.test_after_save = test_after_save;
        }
    }
}

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    conversation::render(view, cx)
}

pub(super) fn error_details(
    view: &MainView,
    error: Option<&str>,
    cx: &mut Context<MainView>,
) -> Div {
    let mut details = v_flex().gap_1();
    if let Some(error) = error {
        details = details.child(
            h_flex().child(
                Button::new("ai-error-details")
                    .label(tr(view.lang(), "ai.error_details"))
                    .ghost()
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ai_form.error_details = !this.ai_form.error_details;
                        cx.notify();
                    })),
            ),
        );
        if view.ai_form.error_details {
            details = details.child(
                div()
                    .text_xs()
                    .whitespace_normal()
                    .text_color(cx.theme().muted_foreground)
                    .child(error.to_owned()),
            );
        }
    }
    details
}

#[cfg(test)]
mod tests;
