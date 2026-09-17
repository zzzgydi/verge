//! Conversation and provider drafts stay separate from daemon snapshots.
mod presentation;
mod settings;

use super::components::{PageHeader, panel};
use crate::{
    ai::{AiCommand, AiOperation, AiSnapshot, ProviderConfig, Secret},
    i18n::{Lang, tr},
    ui::{Page, UiAction, UiResponse},
    view::MainView,
};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
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
    pub settings_open: bool,
    pub advanced_open: bool,
    pub evidence_open: Option<usize>,
    pub clear_key: bool,
    pub confirm_clear: bool,
    pub local_error: Option<String>,
    pub copied: Option<usize>,
    pub error_details: bool,
    pub scroll: ScrollHandle,
    pending_prompt: Option<String>,
    save_requested: bool,
    pending_save: Option<u64>,
    applied: Option<ProviderConfig>,
    provider_dirty: bool,
}

impl AiForm {
    pub fn new(window: &mut Window, cx: &mut Context<MainView>) -> Self {
        let prompt = cx.new(|cx| TextareaState::new(window, cx).submit_on_enter(true));
        cx.subscribe_in(&prompt, window, |this, input, event, window, cx| {
            if this.state.page == Page::Ai
                && !this.ai_form.settings_open
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
            settings_open: false,
            advanced_open: false,
            evidence_open: None,
            clear_key: false,
            confirm_clear: false,
            local_error: None,
            copied: None,
            error_details: false,
            scroll: ScrollHandle::new(),
            pending_prompt: None,
            save_requested: false,
            pending_save: None,
            applied: None,
            provider_dirty: false,
        };
        for field in [
            &form.base,
            &form.model,
            &form.key,
            &form.timeout,
            &form.steps,
        ] {
            cx.subscribe(field, |this, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    this.ai_form.provider_dirty = true;
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
        self.pending_prompt = None;
        self.pending_save = None;
        self.save_requested = false;
    }
    pub fn sync(&mut self, state: &AiSnapshot, window: &mut Window, cx: &mut Context<MainView>) {
        if self.applied.is_some() || self.provider_dirty || state.busy || state.revision == 0 {
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
    fn at_bottom(&self) -> bool {
        self.scroll.max_offset().y + self.scroll.offset().y < px(48.)
    }
}

impl MainView {
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
                if snapshot.revision >= self.state.ai.revision
                    && self.ai_form.at_bottom()
                    && snapshot.messages != self.state.ai.messages
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
                self.ai_form.local_error = Some(error_text(self.lang(), &error.message));
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
        if ai.error.is_none()
            && (ai.operation == Some(AiOperation::Save) || ai.activity == "Settings saved")
        {
            self.ai_form
                .key
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.ai_form.clear_key = false;
            self.ai_form.provider_dirty = false;
            self.ai_form.applied = Some(ai.config.clone());
            self.try_dispatch_ai(AiCommand::TestProvider, cx);
        }
    }
}

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    if view.ai_form.settings_open {
        return settings::render(view, cx);
    }
    let lang = view.lang();
    let ai = &view.state.ai;
    let form = &view.ai_form;
    let busy = ai.busy || view.is_pending(&["ai_write"]);
    let offline = view.state.connection_notice.is_some();
    let ready = !ai.config.base_url.is_empty();
    let mut header = PageHeader::new(tr(lang, "ai.title")).child(
        Button::new("ai-settings")
            .label(tr(lang, "ai.settings"))
            .small()
            .disabled(busy)
            .on_click(cx.listener(|this, _, _, cx| {
                this.ai_form.settings_open = true;
                this.ai_form.local_error = None;
                cx.notify();
            })),
    );
    if !ai.messages.is_empty() {
        header = header.child(
            Button::new("ai-clear")
                .label(tr(lang, "ai.clear"))
                .small()
                .disabled(busy || offline)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.ai_form.confirm_clear = !this.ai_form.confirm_clear;
                    cx.notify();
                })),
        );
    }
    let mut content = v_flex().w_full().gap_5();
    if form.confirm_clear {
        content = content.child(
            panel(cx).gap_2().child(tr(lang, "ai.clear_confirm")).child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("ai-clear-confirm")
                            .label(tr(lang, "ai.clear"))
                            .small()
                            .disabled(busy || offline)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.try_dispatch_ai(AiCommand::Clear, cx) {
                                    this.ai_form.confirm_clear = false;
                                    this.ai_form.evidence_open = None;
                                }
                            })),
                    )
                    .child(
                        Button::new("ai-clear-cancel")
                            .label(tr(lang, "ai.keep"))
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.ai_form.confirm_clear = false;
                                cx.notify();
                            })),
                    ),
            ),
        );
    }
    if ai.messages.is_empty() {
        content = content.child(
            v_flex()
                .py_6()
                .gap_3()
                .child(div().text_2xl().font_semibold().child(tr(
                    lang,
                    if ready {
                        "ai.welcome"
                    } else {
                        "ai.connect_title"
                    },
                )))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr(
                            lang,
                            if ready {
                                "ai.welcome_body"
                            } else {
                                "ai.connect_body"
                            },
                        )),
                ),
        );
        if ready {
            for (index, key) in ["ai.starter_network", "ai.starter_node", "ai.starter_config"]
                .into_iter()
                .enumerate()
            {
                content = content.child(
                    Button::new(("ai-starter", index))
                        .label(tr(lang, key))
                        .disabled(busy || offline)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let prompt = tr(this.lang(), key);
                            this.ai_form.prompt.update(cx, |input, cx| {
                                input.set_value(prompt, window, cx);
                                input.focus(window, cx);
                            });
                            cx.notify();
                        })),
                );
            }
        } else {
            content = content.child(
                div().child(
                    Button::new("ai-connect")
                        .label(tr(lang, "ai.connect"))
                        .primary()
                        .disabled(busy || offline)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.ai_form.settings_open = true;
                            cx.notify();
                        })),
                ),
            );
        }
    }
    for (index, message) in ai.messages.iter().enumerate() {
        let user = message.role == "user";
        let text = message.text.clone();
        let mut block = v_flex().gap_2().min_w_0().w_full();
        let mut title = h_flex()
            .justify_between()
            .gap_2()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(if user { tr(lang, "ai.you") } else { "Verge AI" });
        if !text.is_empty() {
            title = title.child(
                Button::new(("ai-copy", index))
                    .label(tr(
                        lang,
                        if form.copied == Some(index) {
                            "ai.copied"
                        } else {
                            "ai.copy"
                        },
                    ))
                    .ghost()
                    .small()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                        this.ai_form.copied = Some(index);
                        cx.notify();
                    })),
            );
        }
        block = block.child(title);
        if !message.text.is_empty() {
            block = block.child(
                TextView::markdown(
                    ("ai-answer", index),
                    presentation::safe_markdown(&message.text),
                )
                .selectable(true)
                .scrollable(false)
                .on_link_click(|url, _, _, cx| {
                    if url.starts_with("https://") || url.starts_with("http://") {
                        cx.open_url(url);
                    }
                }),
            );
        } else {
            block = block.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr(
                        lang,
                        if ai.busy {
                            "ai.thinking"
                        } else {
                            "ai.no_answer"
                        },
                    )),
            );
        }
        if !user && !message.evidence.is_empty() {
            block = block.child(
                Button::new(("ai-evidence", index))
                    .label(format!(
                        "{} · {}",
                        tr(lang, "ai.evidence"),
                        message.evidence.len()
                    ))
                    .ghost()
                    .small()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ai_form.evidence_open = if this.ai_form.evidence_open == Some(index) {
                            None
                        } else {
                            Some(index)
                        };
                        cx.notify();
                    })),
            );
            if form.evidence_open == Some(index) {
                block = block.child(presentation::evidence_cards(&message.evidence, lang, cx));
            }
        }
        content = content.child(if user {
            block
                .p_3()
                .bg(cx.theme().muted)
                .rounded_lg()
                .into_any_element()
        } else {
            block.px_1().into_any_element()
        });
    }
    let chat_operation = matches!(ai.operation, Some(AiOperation::Start | AiOperation::Retry))
        || ai.operation.is_none();
    if chat_operation && ai.busy {
        content = content.child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(status_text(lang, &ai.activity)),
        );
    }
    let error = form.local_error.clone().or_else(|| {
        chat_operation
            .then(|| ai.error.as_ref().map(|e| error_text(lang, e)))
            .flatten()
    });
    if let Some(error) = error {
        content = content.child(
            panel(cx)
                .gap_2()
                .child(div().text_sm().child(error))
                .child(error_details(view, cx))
                .child(
                    h_flex().gap_2().child(
                        Button::new("ai-error-settings")
                            .small()
                            .label(tr(lang, "ai.settings"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.ai_form.settings_open = true;
                                cx.notify();
                            })),
                    ),
                ),
        );
    }
    if !ai.busy
        && ai.messages.iter().any(|m| m.role == "user")
        && view
            .state
            .daemon_capabilities
            .iter()
            .any(|c| c == crate::ai::UX_CAPABILITY)
    {
        content = content.child(
            div().child(
                Button::new("ai-retry")
                    .label(tr(lang, "ai.retry"))
                    .ghost()
                    .small()
                    .disabled(busy || offline)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ai_form.local_error = None;
                        this.try_dispatch_ai(AiCommand::Retry, cx);
                    })),
            ),
        );
    }
    let mut page = v_flex()
        .h_full()
        .gap_3()
        .child(header)
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(if ready {
                    format!("{} · {}", ai.config.model, tr(lang, "ai.readonly"))
                } else {
                    tr(lang, "ai.readonly").to_owned()
                }),
        )
        .child(
            v_flex()
                .id("ai-conversation")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .track_scroll(&form.scroll)
                .vertical_scrollbar(&form.scroll)
                .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
                .child(
                    h_flex()
                        .w_full()
                        .justify_center()
                        .child(content.max_w(px(800.)).pb_4()),
                ),
        );
    if ready {
        let prompt_empty = form.prompt.read(cx).value().trim().is_empty();
        let mut footer = h_flex().justify_between().gap_2().child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(tr(lang, "ai.keyboard")),
        );
        if !form.at_bottom() {
            footer = footer.child(
                Button::new("ai-latest")
                    .label(tr(lang, "ai.latest"))
                    .ghost()
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ai_form.scroll.scroll_to_bottom();
                        cx.notify();
                    })),
            );
        }
        footer =
            footer.child(if ai.busy {
                Button::new("ai-stop")
                    .debug_selector(|| "ai-stop".into())
                    .label(tr(lang, "ai.stop"))
                    .small()
                    .disabled(offline)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.dispatch(UiAction::Ai(AiCommand::Cancel), cx)
                    }))
            } else {
                Button::new("ai-send")
                    .debug_selector(|| "ai-send".into())
                    .label(tr(lang, "ai.send"))
                    .primary()
                    .small()
                    .disabled(busy || offline || prompt_empty)
                    .on_click(cx.listener(|this, _, window, cx| this.send_ai(window, cx)))
            });
        page = page.child(
            v_flex()
                .flex_shrink_0()
                .gap_2()
                .child(
                    Textarea::new(&form.prompt)
                        .h(px(76.))
                        .aria_label(tr(lang, "ai.prompt")),
                )
                .child(footer),
        );
    }
    page.child(
        div()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(tr(lang, "ai.privacy_short")),
    )
    .into_any_element()
}

fn error_details(view: &MainView, cx: &mut Context<MainView>) -> Div {
    let mut details = v_flex().gap_1();
    if let Some(error) = &view.state.ai.error {
        details = details.child(
            div().child(
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
                    .child(error.clone()),
            );
        }
    }
    details
}

#[cfg(test)]
mod tests;
