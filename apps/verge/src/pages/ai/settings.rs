use super::*;

fn field(label: &str, input: &Entity<InputState>, disabled: bool, cx: &App) -> Div {
    v_flex()
        .flex_1()
        .min_w_0()
        .gap_1()
        .child(div().text_sm().child(label.to_owned()))
        .child(Input::new(input).disabled(disabled))
        .text_color(cx.theme().foreground)
}

pub(super) fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let lang = view.lang();
    let ai = &view.state.ai;
    let form = &view.ai_form;
    let busy = ai.busy || view.is_pending(&["ai_write"]);
    let offline = view.state.connection_notice.is_some();
    let mut content = panel(cx)
        .w_full()
        .max_w(px(640.))
        .gap_3()
        .child(
            div()
                .text_lg()
                .font_semibold()
                .child(tr(lang, "ai.provider_title")),
        )
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(tr(lang, "ai.provider_body")),
        )
        .child(field(tr(lang, "ai.base"), &form.base, busy, cx))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(tr(lang, "ai.base_hint")),
        )
        .child(field(tr(lang, "ai.model"), &form.model, busy, cx))
        .child(field(
            if ai.has_key {
                tr(lang, "ai.key_saved")
            } else {
                "API key"
            },
            &form.key,
            busy,
            cx,
        ));
    if ai.has_key {
        content = content.child(
            Button::new("ai-clear-key")
                .label(tr(
                    lang,
                    if form.clear_key {
                        "ai.key_remove_pending"
                    } else {
                        "ai.clear_key"
                    },
                ))
                .ghost()
                .small()
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.ai_form.clear_key = !this.ai_form.clear_key;
                    cx.notify();
                })),
        );
    }
    content = content.child(
        Button::new("ai-advanced")
            .label(tr(
                lang,
                if form.advanced_open {
                    "ai.advanced_hide"
                } else {
                    "ai.advanced"
                },
            ))
            .ghost()
            .small()
            .on_click(cx.listener(|this, _, _, cx| {
                this.ai_form.advanced_open = !this.ai_form.advanced_open;
                cx.notify();
            })),
    );
    if form.advanced_open {
        content = content.child(
            h_flex()
                .gap_3()
                .child(field(tr(lang, "ai.timeout"), &form.timeout, busy, cx))
                .child(field(tr(lang, "ai.steps"), &form.steps, busy, cx)),
        );
    }
    let operation_is_settings = matches!(ai.operation, Some(AiOperation::Save | AiOperation::Test));
    let mut footer = h_flex()
        .flex_shrink_0()
        .gap_2()
        .child(
            Button::new("ai-save")
                .debug_selector(|| "ai-save".into())
                .label(tr(lang, "ai.save_test"))
                .primary()
                .disabled(busy || offline)
                .on_click(cx.listener(|this, _, _, cx| {
                    let config = ProviderConfig {
                        base_url: this.ai_form.base.read(cx).value().to_string(),
                        model: this.ai_form.model.read(cx).value().to_string(),
                        timeout_seconds: this.ai_form.timeout.read(cx).value().parse().unwrap_or(0),
                        max_tool_steps: this.ai_form.steps.read(cx).value().parse().unwrap_or(0),
                    };
                    if let Err(error) = config.clone().validated() {
                        this.ai_form.local_error = Some(error_text(this.lang(), &error.message));
                        cx.notify();
                        return;
                    }
                    this.ai_form.local_error = None;
                    if this.try_dispatch_ai(
                        AiCommand::SaveConfig {
                            config,
                            api_key: Secret(this.ai_form.key.read(cx).value().to_string()),
                            clear_key: this.ai_form.clear_key,
                        },
                        cx,
                    ) {
                        this.ai_form.save_requested = true;
                    }
                })),
        )
        .child(
            Button::new("ai-settings-done")
                .label(tr(lang, "ai.back"))
                .disabled(busy)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.ai_form.settings_open = false;
                    this.ai_form.local_error = None;
                    this.ai_form
                        .prompt
                        .update(cx, |input, cx| input.focus(window, cx));
                    cx.notify();
                })),
        );
    if ai.busy && ai.operation == Some(AiOperation::Test) {
        footer =
            footer.child(
                Button::new("ai-settings-stop")
                    .label(tr(lang, "ai.stop"))
                    .disabled(offline)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.dispatch(UiAction::Ai(AiCommand::Cancel), cx)
                    })),
            );
    }
    let mut feedback = v_flex().flex_shrink_0().gap_2();
    if operation_is_settings {
        feedback = feedback.child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(status_text(lang, &ai.activity)),
        );
    }
    if let Some(error) = form.local_error.clone().or_else(|| {
        operation_is_settings
            .then(|| ai.error.as_ref().map(|e| error_text(lang, e)))
            .flatten()
    }) {
        feedback = feedback
            .child(div().text_sm().text_color(cx.theme().danger).child(error))
            .child(error_details(view, cx));
    }
    v_flex()
        .h_full()
        .gap_4()
        .child(PageHeader::new(tr(lang, "ai.settings")))
        .child(
            v_flex()
                .id("ai-settings-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .child(h_flex().w_full().justify_center().child(content)),
        )
        .child(feedback)
        .child(footer)
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(tr(lang, "ai.scope")),
        )
        .into_any_element()
}
