use super::*;
use gpui_kit::component::{Icon, IconName};

const CHAT_WIDTH: f32 = 780.;

pub(super) fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let lang = view.lang();
    let ai = &view.state.ai;
    let form = &view.ai_form;
    let busy = ai.busy || view.is_pending(&["ai_write"]);
    let offline = view.state.connection_notice.is_some();
    let ready = !ai.config.base_url.is_empty();
    let chat_operation = matches!(
        ai.operation,
        Some(AiOperation::Start | AiOperation::Retry | AiOperation::Approve | AiOperation::Dismiss)
    ) || ai.operation.is_none();
    let mut header = PageHeader::new(tr(lang, "ai.title"));
    if ready {
        header = header.child(
            div()
                .max_w(px(200.))
                .truncate()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(ai.config.model.clone()),
        );
    }
    header = header
        .child(
            Button::new("ai-data-scope")
                .debug_selector(|| "ai-data-scope".into())
                .icon(IconName::Info)
                .tooltip(tr(lang, "ai.data_scope"))
                .ghost()
                .small()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.ai_form.data_scope_open = !this.ai_form.data_scope_open;
                    cx.notify();
                })),
        )
        .child(
            Button::new("ai-settings")
                .debug_selector(|| "ai-settings".into())
                .icon(IconName::Settings2)
                .tooltip(tr(lang, "ai.settings"))
                .ghost()
                .small()
                .on_click(cx.listener(|this, _, _, cx| this.open_ai_settings(cx))),
        );
    if !ai.messages.is_empty() {
        header = header.child(
            Button::new("ai-clear")
                .debug_selector(|| "ai-clear".into())
                .icon(IconName::Plus)
                .label(tr(lang, "ai.clear"))
                .small()
                .ghost()
                .disabled(busy || offline)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.ai_form.confirm_clear = !this.ai_form.confirm_clear;
                    cx.notify();
                })),
        );
    }
    let clear_confirmation = form.confirm_clear.then(|| {
        panel(cx)
            .flex_shrink_0()
            .gap_3()
            .child(div().text_sm().child(tr(lang, "ai.clear_confirm")))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("ai-clear-confirm")
                            .debug_selector(|| "ai-clear-confirm".into())
                            .label(tr(lang, "ai.clear"))
                            .primary()
                            .small()
                            .disabled(busy || offline)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.try_dispatch_ai(AiCommand::Clear, cx);
                            })),
                    )
                    .child(
                        Button::new("ai-clear-cancel")
                            .label(tr(lang, "common.cancel"))
                            .ghost()
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.ai_form.confirm_clear = false;
                                cx.notify();
                            })),
                    ),
            )
    });
    let mut content = v_flex().w_full().min_w_0().gap_6();
    if ai.messages.is_empty() && ai.proposals.is_empty() {
        let mut welcome = v_flex()
            .w_full()
            .gap_4()
            .py_6()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size_10()
                    .rounded_xl()
                    .bg(cx.theme().muted)
                    .child(Icon::new(IconName::Bot).size_5()),
            )
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
                    .max_w(px(520.))
                    .whitespace_normal()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr(
                        lang,
                        if ready {
                            "ai.welcome_body"
                        } else {
                            "ai.connect_body"
                        },
                    )),
            );
        if ready {
            let mut starters = h_flex().w_full().gap_3().items_stretch();
            for (index, (icon, title, prompt)) in [
                (
                    IconName::Globe,
                    "ai.starter_network_title",
                    "ai.starter_network",
                ),
                (
                    IconName::Network,
                    "ai.starter_node_title",
                    "ai.starter_node",
                ),
                (
                    IconName::FileText,
                    "ai.starter_config_title",
                    "ai.starter_config",
                ),
            ]
            .into_iter()
            .enumerate()
            {
                starters = starters.child(
                    Button::new(("ai-starter", index))
                        .debug_selector(move || format!("ai-starter-{index}"))
                        .flex_1()
                        .min_w_0()
                        .h(px(62.))
                        .icon(icon)
                        .label(tr(lang, title))
                        .tooltip(tr(lang, prompt))
                        .disabled(busy || offline)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let value = tr(this.lang(), prompt);
                            this.ai_form.prompt.update(cx, |input, cx| {
                                input.set_value(value, window, cx);
                                input.focus(window, cx);
                            });
                            this.ai_form.local_error = None;
                            cx.notify();
                        })),
                );
            }
            welcome = welcome.child(starters);
        } else {
            welcome = welcome.child(
                h_flex().child(
                    Button::new("ai-connect")
                        .debug_selector(|| "ai-connect".into())
                        .label(tr(lang, "ai.connect"))
                        .primary()
                        .on_click(cx.listener(|this, _, _, cx| this.open_ai_settings(cx))),
                ),
            );
        }
        content = content.child(welcome);
    }
    for (index, message) in ai.messages.iter().enumerate() {
        let user = message.role == "user";
        let active = chat_operation && ai.busy && index + 1 == ai.messages.len();
        let mut block = v_flex().min_w_0().gap_2();
        if user {
            block = block
                .relative()
                .max_w(px(620.))
                .px_4()
                .pr_12()
                .py_3()
                .rounded_xl()
                .bg(cx.theme().muted)
                .child(
                    div()
                        .text_sm()
                        .whitespace_normal()
                        .child(message.text.clone()),
                );
        } else {
            block = block
                .flex_1()
                .child(div().text_sm().font_medium().child("Verge"));
            if message.text.is_empty() {
                block = block.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr(
                            lang,
                            if active {
                                "ai.thinking"
                            } else if ai.activity == "Cancelled" {
                                "ai.stopped"
                            } else {
                                "ai.no_answer"
                            },
                        )),
                );
            } else {
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
            }
        }
        let mut actions = h_flex().gap_1();
        if !message.text.is_empty() && !active {
            let text = message.text.clone();
            actions = actions.child(
                Button::new(("ai-copy", index))
                    .debug_selector(move || format!("ai-copy-{index}"))
                    .icon(if form.copied == Some(index) {
                        IconName::Check
                    } else {
                        IconName::Copy
                    })
                    .tooltip(tr(
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
        if !user && !message.evidence.is_empty() {
            actions = actions.child(
                Button::new(("ai-evidence", index))
                    .debug_selector(move || format!("ai-evidence-{index}"))
                    .icon(if form.evidence_open == Some(index) {
                        IconName::ChevronUp
                    } else {
                        IconName::ChevronDown
                    })
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
        }
        block = block.child(if user {
            actions.absolute().top_2().right_2()
        } else {
            actions
        });
        if !user && form.evidence_open == Some(index) {
            block = block.child(presentation::evidence_cards(&message.evidence, lang, cx));
        }
        content = content.child(if user {
            h_flex()
                .w_full()
                .min_w_0()
                .justify_end()
                .child(block)
                .into_any_element()
        } else {
            h_flex()
                .w_full()
                .min_w_0()
                .items_start()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .size_8()
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .rounded_lg()
                        .border_1()
                        .border_color(cx.theme().border)
                        .child(Icon::new(IconName::Bot).size_4()),
                )
                .child(block)
                .into_any_element()
        });
    }
    content = content.child(proposals::render(view, cx));
    if chat_operation && ai.busy {
        content = content.child(
            h_flex()
                .gap_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(IconName::LoaderCircle).size_3())
                .child(if form.stop_requested {
                    tr(lang, "ai.stopping").to_owned()
                } else {
                    status_text(lang, &ai.activity)
                }),
        );
    } else if chat_operation && ai.activity == "Cancelled" {
        content = content.child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(tr(lang, "ai.stopped")),
        );
    }
    let error = form.local_error.clone().or_else(|| {
        if chat_operation {
            ai.error
                .as_ref()
                .filter(|e| e.as_str() != "Cancelled")
                .map(|e| error_text(lang, e))
        } else {
            None
        }
    });
    if let Some(error) = error {
        content = content.child(
            v_flex()
                .debug_selector(|| "ai-chat-error".into())
                .gap_2()
                .border_l_2()
                .border_color(cx.theme().danger)
                .pl_3()
                .child(div().text_sm().whitespace_normal().child(error))
                .child(error_details(
                    view,
                    ai.error
                        .as_deref()
                        .filter(|_| chat_operation && form.local_error.is_none()),
                    cx,
                )),
        );
    }
    if !busy
        && ai.messages.iter().any(|m| m.role == "user")
        && view
            .state
            .daemon_capabilities
            .iter()
            .any(|c| c == crate::ai::UX_CAPABILITY)
    {
        content = content.child(
            h_flex().child(
                Button::new("ai-retry")
                    .debug_selector(|| "ai-retry".into())
                    .icon(IconName::RotateCw)
                    .label(tr(lang, "ai.retry"))
                    .ghost()
                    .small()
                    .disabled(offline || !ready)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ai_form.local_error = None;
                        this.ai_form.copied = None;
                        this.ai_form.evidence_open = None;
                        this.try_dispatch_ai(AiCommand::Retry, cx);
                    })),
            ),
        );
    }
    let mut page = v_flex()
        .relative()
        .h_full()
        .min_h_0()
        .gap_4()
        .child(header)
        .children(clear_confirmation)
        .child(
            v_flex()
                .id("ai-conversation")
                .debug_selector(|| "ai-conversation".into())
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_y_scroll()
                .track_scroll(&form.scroll)
                .vertical_scrollbar(&form.scroll)
                .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .justify_center()
                        .child(content.max_w(px(CHAT_WIDTH)).py_3().px_2()),
                ),
        );
    if ready {
        let mut toolbar = h_flex().justify_between().gap_2().child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(tr(lang, "ai.keyboard")),
        );
        let mut actions = h_flex().gap_2();
        if !form.at_bottom() {
            actions = actions.child(
                Button::new("ai-latest")
                    .icon(IconName::ArrowDown)
                    .tooltip(tr(lang, "ai.latest"))
                    .small()
                    .ghost()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ai_form.scroll.scroll_to_bottom();
                        cx.notify();
                    })),
            );
        }
        actions = actions.child(if ai.busy {
            Button::new("ai-stop")
                .debug_selector(|| "ai-stop".into())
                .icon(IconName::Pause)
                .label(tr(
                    lang,
                    if form.stop_requested {
                        "ai.stopping"
                    } else {
                        "ai.stop"
                    },
                ))
                .small()
                .disabled(offline || form.stop_requested)
                .on_click(cx.listener(|this, _, _, cx| this.stop_ai(cx)))
        } else {
            Button::new("ai-send")
                .debug_selector(|| "ai-send".into())
                .icon(IconName::ArrowUp)
                .tooltip(tr(lang, "ai.send"))
                .primary()
                .small()
                .disabled(busy || offline || form.prompt.read(cx).value().trim().is_empty())
                .on_click(cx.listener(|this, _, window, cx| this.send_ai(window, cx)))
        });
        toolbar = toolbar.child(actions);
        page = page.child(
            h_flex().w_full().justify_center().flex_shrink_0().child(
                v_flex()
                    .debug_selector(|| "ai-composer".into())
                    .w_full()
                    .max_w(px(CHAT_WIDTH))
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded_xl()
                    .p_3()
                    .gap_2()
                    .child(
                        Textarea::new(&form.prompt)
                            .h(px(64.))
                            .appearance(false)
                            .bordered(false)
                            .aria_label(tr(lang, "ai.prompt")),
                    )
                    .child(toolbar),
            ),
        );
    }
    if form.data_scope_open {
        page = page.child(
            panel(cx)
                .absolute()
                .top(px(44.))
                .right_0()
                .w_full()
                .max_w(px(560.))
                .h(px(208.))
                .gap_2()
                .shadow_md()
                .child(
                    h_flex()
                        .flex_shrink_0()
                        .justify_between()
                        .child(
                            div()
                                .text_sm()
                                .font_medium()
                                .child(tr(lang, "ai.data_scope")),
                        )
                        .child(
                            Button::new("ai-scope-close")
                                .icon(IconName::Close)
                                .tooltip(tr(lang, "ai.close_details"))
                                .ghost()
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.ai_form.data_scope_open = false;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .id("ai-scope-scroll")
                        .debug_selector(|| "ai-scope-body".into())
                        .flex_1()
                        .overflow_y_scrollbar()
                        .min_h_0()
                        .text_xs()
                        .whitespace_normal()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr(lang, "ai.data_scope_detail")),
                ),
        );
    }
    page.into_any_element()
}
