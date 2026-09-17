use super::components::{PageHeader, panel};
use crate::{
    ai::{AiCommand, AiSnapshot, ProviderConfig, Secret},
    i18n::tr,
    ui::UiAction,
    view::MainView,
};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState, Textarea, TextareaState},
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::*;

pub struct AiForm {
    pub base: Entity<InputState>,
    pub model: Entity<InputState>,
    pub key: Entity<InputState>,
    pub timeout: Entity<InputState>,
    pub steps: Entity<InputState>,
    pub prompt: Entity<TextareaState>,
    pub settings_open: bool,
    pub evidence_open: bool,
    applied: Option<ProviderConfig>,
}

impl AiForm {
    pub fn new(window: &mut Window, cx: &mut Context<MainView>) -> Self {
        Self {
            base: cx
                .new(|cx| InputState::new(window, cx).placeholder("https://provider.example/v1")),
            model: cx.new(|cx| InputState::new(window, cx).placeholder("Model ID")),
            key: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder("API key")
            }),
            timeout: cx.new(|cx| InputState::new(window, cx).default_value("60")),
            steps: cx.new(|cx| InputState::new(window, cx).default_value("4")),
            prompt: cx.new(|cx| TextareaState::new(window, cx)),
            settings_open: false,
            evidence_open: false,
            applied: None,
        }
    }
    pub fn reset_sync(&mut self) {
        self.applied = None;
    }
    pub fn sync(&mut self, state: &AiSnapshot, window: &mut Window, cx: &mut Context<MainView>) {
        if self.applied.as_ref() == Some(&state.config) {
            return;
        }
        // Background snapshots never replace focused provider drafts.
        if [&self.base, &self.model, &self.timeout, &self.steps]
            .iter()
            .any(|field| field.read(cx).focus_handle(cx).is_focused(window))
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
}

fn form_field(label: &str, input: &Entity<InputState>, cx: &App) -> Div {
    v_flex()
        .flex_1()
        .min_w_0()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label.to_owned()),
        )
        .child(Input::new(input).small())
}

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let lang = view.lang();
    let ai = &view.state.ai;
    let busy = ai.busy || view.is_pending(&["ai_write"]);
    let offline = view.state.connection_notice.is_some();
    let form = &view.ai_form;
    let show_settings = form.settings_open || ai.config.base_url.is_empty();
    let header = PageHeader::new(tr(lang, "ai.title"))
        .child(
            Button::new("ai-settings")
                .label(tr(lang, "ai.settings"))
                .small()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.ai_form.settings_open = !this.ai_form.settings_open;
                    cx.notify();
                })),
        )
        .child(
            Button::new("ai-clear")
                .label(tr(lang, "ai.clear"))
                .small()
                .disabled(busy || offline)
                .on_click(
                    cx.listener(|this, _, _, cx| this.dispatch(UiAction::Ai(AiCommand::Clear), cx)),
                ),
        );
    let mut content = v_flex().gap_4();
    if show_settings {
        content = content.child(
            panel(cx)
                .gap_3()
                .child(form_field("Base URL", &form.base, cx))
                .child(
                    h_flex()
                        .gap_3()
                        .child(form_field(tr(lang, "ai.model"), &form.model, cx))
                        .child(form_field(
                            if ai.has_key {
                                tr(lang, "ai.key_saved")
                            } else {
                                "API key"
                            },
                            &form.key,
                            cx,
                        )),
                )
                .child(
                    h_flex()
                        .gap_3()
                        .child(form_field(tr(lang, "ai.timeout"), &form.timeout, cx))
                        .child(form_field(tr(lang, "ai.steps"), &form.steps, cx)),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("ai-save")
                                .label(tr(lang, "ai.save"))
                                .primary()
                                .small()
                                .disabled(busy || offline)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    let config = ProviderConfig {
                                        base_url: this.ai_form.base.read(cx).value().to_string(),
                                        model: this.ai_form.model.read(cx).value().to_string(),
                                        timeout_seconds: this
                                            .ai_form
                                            .timeout
                                            .read(cx)
                                            .value()
                                            .parse()
                                            .unwrap_or(0),
                                        max_tool_steps: this
                                            .ai_form
                                            .steps
                                            .read(cx)
                                            .value()
                                            .parse()
                                            .unwrap_or(0),
                                    };
                                    if let Err(error) = config.clone().validated() {
                                        this.state.set_error(error);
                                        cx.notify();
                                        return;
                                    }
                                    let api_key =
                                        Secret(this.ai_form.key.read(cx).value().to_string());
                                    this.dispatch(
                                        UiAction::Ai(AiCommand::SaveConfig {
                                            config,
                                            api_key,
                                            clear_key: false,
                                        }),
                                        cx,
                                    );
                                    this.ai_form
                                        .key
                                        .update(cx, |input, cx| input.set_value("", window, cx));
                                })),
                        )
                        .child(
                            Button::new("ai-test")
                                .label(tr(lang, "ai.test"))
                                .small()
                                .disabled(busy || offline || ai.config.base_url.is_empty())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.dispatch(UiAction::Ai(AiCommand::TestProvider), cx)
                                })),
                        )
                        .child(
                            Button::new("ai-clear-key")
                                .label(tr(lang, "ai.clear_key"))
                                .small()
                                .disabled(busy || offline || !ai.has_key)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.dispatch(
                                        UiAction::Ai(AiCommand::SaveConfig {
                                            config: this.state.ai.config.clone(),
                                            api_key: Secret::default(),
                                            clear_key: true,
                                        }),
                                        cx,
                                    )
                                })),
                        ),
                ),
        );
    }
    content = content.child(
        div()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(tr(lang, "ai.scope")),
    );
    if ai.messages.is_empty() {
        content = content.child(panel(cx).child(tr(lang, "ai.empty")));
    }
    for (index, message) in ai.messages.iter().enumerate() {
        content = content.child(
            panel(cx)
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(if message.role == "user" {
                            tr(lang, "ai.you")
                        } else {
                            "Verge AI"
                        }),
                )
                // Plain text: model output cannot create remote images or active links.
                .child(div().id(("ai-message", index)).whitespace_normal().child(
                    if message.text.is_empty() {
                        "…".to_owned()
                    } else {
                        message.text.clone()
                    },
                )),
        );
    }
    if !ai.activity.is_empty() {
        content = content.child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(ai.activity.clone()),
        );
    }
    if let Some(error) = &ai.error {
        content = content.child(
            div()
                .text_sm()
                .text_color(cx.theme().danger)
                .child(error.clone()),
        );
        if let Some(prompt) = ai
            .messages
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map(|message| message.text.clone())
        {
            content = content.child(
                Button::new("ai-retry")
                    .label(tr(lang, "ai.retry"))
                    .small()
                    .disabled(busy || offline)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.dispatch(
                            UiAction::Ai(AiCommand::Start {
                                prompt: prompt.clone(),
                            }),
                            cx,
                        )
                    })),
            );
        }
    }
    if !ai.evidence.is_empty() {
        content = content.child(
            Button::new("ai-evidence")
                .label(tr(lang, "ai.evidence"))
                .small()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.ai_form.evidence_open = !this.ai_form.evidence_open;
                    cx.notify();
                })),
        );
        if form.evidence_open {
            for evidence in &ai.evidence {
                content = content.child(
                    panel(cx)
                        .gap_1()
                        .child(format!(
                            "{} · {} · {}",
                            evidence.id, evidence.source, evidence.captured_at
                        ))
                        .child(
                            div()
                                .text_xs()
                                .whitespace_normal()
                                .child(evidence.data.to_string()),
                        ),
                );
            }
        }
    }
    let conversation = content;
    v_flex()
        .h_full()
        .gap_3()
        .child(header)
        .child(
            v_flex()
                .id("ai-conversation")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .child(conversation),
        )
        .child(Textarea::new(&form.prompt).h(px(80.)).flex_shrink_0())
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("ai-send")
                        .debug_selector(|| "ai-send".into())
                        .label(tr(lang, "ai.send"))
                        .primary()
                        .small()
                        .disabled(busy || offline || ai.config.base_url.is_empty())
                        .on_click(cx.listener(|this, _, window, cx| {
                            let prompt = this.ai_form.prompt.read(cx).value().to_string();
                            if prompt.trim().is_empty() {
                                return;
                            }
                            this.dispatch(UiAction::Ai(AiCommand::Start { prompt }), cx);
                            this.ai_form
                                .prompt
                                .update(cx, |input, cx| input.set_value("", window, cx));
                        })),
                )
                .child(
                    Button::new("ai-stop")
                        .debug_selector(|| "ai-stop".into())
                        .label(tr(lang, "ai.stop"))
                        .small()
                        .disabled(!ai.busy || offline)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.dispatch(UiAction::Ai(AiCommand::Cancel), cx)
                        })),
                ),
        )
        .into_any_element()
}
