use crate::{
    ai::AiOperation,
    i18n::tr,
    pages::ai::{
        error_details,
        presentation::{error_text, status_text},
    },
    ui::Page,
    view::MainView,
};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    scroll::ScrollableElement as _,
    v_flex,
};
use gpui_kit::*;

fn field(label: &str, input: &Entity<InputState>, disabled: bool) -> Div {
    v_flex()
        .flex_1()
        .min_w_0()
        .gap_2()
        .child(
            h_flex()
                .h_7()
                .text_sm()
                .font_medium()
                .child(label.to_owned()),
        )
        .child(Input::new(input).disabled(disabled))
}

pub(super) fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let lang = view.lang();
    let ai = &view.state.ai;
    let form = &view.ai_form;
    let busy = ai.busy || view.is_pending(&["ai_write"]);
    let offline = view.state.connection_notice.is_some();
    let dirty = form.has_provider_draft(cx);
    let mut content = v_flex()
        .w_full()
        .max_w(px(660.))
        .gap_4()
        .py_1()
        .child(
            h_flex()
                .justify_between()
                .gap_3()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(
                            div()
                                .text_base()
                                .font_medium()
                                .child(tr(lang, "ai.provider_title")),
                        )
                        .child(
                            div()
                                .text_xs()
                                .whitespace_normal()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr(lang, "ai.provider_body")),
                        ),
                )
                .child(
                    Button::new("ai-settings-done")
                        .flex_shrink_0()
                        .icon(IconName::ArrowLeft)
                        .label(tr(lang, "ai.back"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.navigate(Page::Ai, cx);
                            this.ai_form
                                .prompt
                                .update(cx, |input, cx| input.focus(window, cx));
                        })),
                ),
        )
        .child(
            v_flex()
                .gap_2()
                .child(field(tr(lang, "ai.base"), &form.base, busy))
                .child(
                    div()
                        .text_xs()
                        .whitespace_normal()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr(lang, "ai.base_hint")),
                ),
        );
    let mut key_label = h_flex()
        .h_7()
        .justify_between()
        .gap_2()
        .child(div().text_sm().font_medium().child("API Key"));
    if ai.has_key {
        key_label = key_label.child(
            Button::new("ai-clear-key")
                .debug_selector(|| "ai-clear-key".into())
                .label(tr(
                    lang,
                    if form.clear_key {
                        "ai.undo_clear_key"
                    } else {
                        "ai.clear_key"
                    },
                ))
                .small()
                .ghost()
                .disabled(busy)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.ai_form.clear_key = !this.ai_form.clear_key;
                    this.ai_form.settings_error = None;
                    if this.ai_form.clear_key {
                        this.ai_form
                            .key
                            .update(cx, |input, cx| input.set_value("", window, cx));
                    }
                    cx.notify();
                })),
        );
    }
    let key = v_flex()
        .flex_1()
        .min_w_0()
        .gap_2()
        .child(key_label)
        .child(Input::new(&form.key).disabled(busy))
        .child(
            div()
                .debug_selector(|| "ai-key-hint".into())
                .text_xs()
                .whitespace_normal()
                .text_color(cx.theme().muted_foreground)
                .child(tr(
                    lang,
                    if form.clear_key {
                        "ai.key_will_clear"
                    } else if ai.has_key {
                        "ai.key_keep_hint"
                    } else {
                        "ai.key_optional"
                    },
                )),
        );
    content = content.child(
        h_flex()
            .items_start()
            .gap_4()
            .child(field(tr(lang, "ai.model"), &form.model, busy))
            .child(key),
    );
    content = content.child(
        h_flex()
            .border_t_1()
            .border_color(cx.theme().border)
            .pt_2()
            .child(
                Button::new("ai-advanced")
                    .debug_selector(|| "ai-advanced".into())
                    .icon(if form.advanced_open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .label(tr(lang, "ai.advanced"))
                    .ghost()
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ai_form.advanced_open = !this.ai_form.advanced_open;
                        cx.notify();
                    })),
            ),
    );
    if form.advanced_open {
        content = content.child(
            h_flex()
                .gap_4()
                .child(field(tr(lang, "ai.timeout"), &form.timeout, busy))
                .child(field(tr(lang, "ai.steps"), &form.steps, busy)),
        );
    }
    let settings_operation = matches!(
        ai.operation,
        Some(AiOperation::Save | AiOperation::Test | AiOperation::State)
    );
    let error = form.settings_error.clone().or_else(|| {
        if settings_operation {
            ai.error
                .as_ref()
                .filter(|e| e.as_str() != "Cancelled")
                .map(|e| error_text(lang, e))
        } else {
            None
        }
    });
    let feedback = error.map(|error| {
        let mut feedback = v_flex().w_full().max_w(px(660.)).gap_2().child(
            v_flex()
                .debug_selector(|| "ai-settings-error".into())
                .gap_2()
                .border_l_2()
                .border_color(cx.theme().danger)
                .pl_3()
                .child(div().text_sm().whitespace_normal().child(error))
                .child(error_details(
                    view,
                    ai.error
                        .as_deref()
                        .filter(|_| settings_operation && form.settings_error.is_none()),
                    cx,
                )),
        );
        if ai.operation == Some(AiOperation::State) && !busy {
            feedback = feedback.child(
                h_flex().child(
                    Button::new("ai-reload-settings")
                        .debug_selector(|| "ai-reload-settings".into())
                        .icon(IconName::RotateCw)
                        .label(tr(lang, "ai.reload_settings"))
                        .small()
                        .disabled(offline)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.ai_form.settings_error = None;
                            this.try_dispatch_ai(crate::ai::AiCommand::GetState, cx);
                        })),
                ),
            );
        }
        feedback
    });
    let mut footer = h_flex().w_full().max_w(px(660.)).justify_between().gap_3();
    let status = if busy && settings_operation {
        if form.stop_requested {
            tr(lang, "ai.stopping").into()
        } else if ai.operation == Some(AiOperation::Save) {
            tr(lang, "ai.saving").into()
        } else if ai.operation == Some(AiOperation::State) {
            tr(lang, "ai.loading_settings").into()
        } else {
            tr(lang, "ai.testing").into()
        }
    } else if dirty {
        tr(lang, "ai.unsaved").into()
    } else if settings_operation && ai.error.is_none() && !ai.activity.is_empty() {
        status_text(lang, &ai.activity)
    } else {
        String::new()
    };
    footer = footer.child(
        div()
            .flex_1()
            .min_w_0()
            .text_xs()
            .whitespace_normal()
            .text_color(cx.theme().muted_foreground)
            .child(status),
    );
    let mut actions = h_flex().flex_shrink_0().gap_2();
    if ai.busy && ai.operation == Some(AiOperation::Test) {
        actions = actions.child(
            Button::new("ai-settings-stop")
                .debug_selector(|| "ai-settings-stop".into())
                .icon(Icon::new(IconName::Pause).size_4())
                .label(tr(lang, "ai.stop"))
                .disabled(offline || form.stop_requested)
                .on_click(cx.listener(|this, _, _, cx| this.stop_ai(cx))),
        );
    } else {
        actions = actions.child(
            Button::new("ai-test")
                .debug_selector(|| "ai-test".into())
                .label(tr(lang, "ai.test_connection"))
                .tooltip(tr(lang, "ai.test_hint"))
                .disabled(busy || offline)
                .on_click(cx.listener(|this, _, _, cx| this.save_ai_settings(true, cx))),
        );
    }
    actions = actions.child(
        Button::new("ai-save")
            .debug_selector(|| "ai-save".into())
            .label(tr(lang, "common.save"))
            .primary()
            .loading(ai.busy && ai.operation == Some(AiOperation::Save))
            .disabled(busy || offline)
            .on_click(cx.listener(|this, _, _, cx| this.save_ai_settings(false, cx))),
    );
    footer = footer.child(actions);
    v_flex()
        .flex_1()
        .min_h_0()
        .gap_4()
        .child(
            v_flex()
                .id("ai-settings-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .child(h_flex().w_full().justify_center().child(content)),
        )
        .children(feedback.map(|feedback| {
            h_flex()
                .w_full()
                .justify_center()
                .flex_shrink_0()
                .child(feedback)
        }))
        .child(
            h_flex()
                .w_full()
                .justify_center()
                .flex_shrink_0()
                .pt_3()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(footer),
        )
        .into_any_element()
}
