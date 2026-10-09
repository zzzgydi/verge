use super::*;
use gpui_kit::component::{Icon, IconName};

pub(super) fn render(view: &MainView, cx: &mut Context<MainView>) -> Div {
    let lang = view.lang();
    let available = view
        .state
        .daemon_capabilities
        .iter()
        .any(|c| c == crate::ai::ACTIONS_CAPABILITY);
    let disabled = view.state.ai.busy
        || view.is_pending(&["ai_write"])
        || view.state.connection_notice.is_some()
        || !available;
    let mut list = v_flex().gap_3();
    for (index, proposal) in view.state.ai.proposals.iter().enumerate() {
        let pending =
            proposal.status == "pending" && proposal.expires_at > crate::ai::proposals::now();
        let confirming = view.ai_form.confirm_proposal.as_ref() == Some(&proposal.id);
        let mut card = panel(cx)
            .gap_3()
            .min_w_0()
            .child(
                h_flex()
                    .gap_2()
                    .child(Icon::new(IconName::FileText).size_4())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_semibold()
                            .whitespace_normal()
                            .child(format!(
                                "{} · {}",
                                tr(lang, "ai.proposal"),
                                if proposal.kind == "proxy_settings" {
                                    tr(lang, "ai.proxy_settings_title")
                                } else {
                                    &proposal.target
                                }
                            )),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{} · {}",
                        proposal.evidence_id,
                        tr(
                            lang,
                            match proposal.status.as_str() {
                                "applied" => "ai.applied",
                                "failed" | "recovery_failed" => "ai.apply_failed",
                                "rejected" => "ai.rejected",
                                "restored" => "ai.restored",
                                "dismissed" => "ai.dismissed",
                                "pending" if pending => "ai.awaiting_approval",
                                _ => "ai.expired",
                            }
                        )
                    )),
            );
        for change in &proposal.changes {
            let label = match change.path.as_str() {
                "Added Merge operations" => tr(lang, "ai.merge_operations"),
                "scope" => tr(lang, "ai.change_scope"),
                "proxy.bypass" => tr(lang, "ai.proxy_bypass"),
                "proxy.effective_bypass" => tr(lang, "ai.proxy_effective_bypass"),
                "proxy.use_default_bypass" => tr(lang, "ai.proxy_defaults"),
                "proxy.pac_mode" => tr(lang, "ai.proxy_pac"),
                "proxy.guard_enabled" => tr(lang, "ai.proxy_guard"),
                "proxy.guard_interval_secs" => tr(lang, "ai.proxy_guard_interval"),
                _ => &change.path,
            };
            let mut change_view = v_flex()
                .min_w_0()
                .gap_2()
                .p_3()
                .rounded_lg()
                .bg(cx.theme().muted)
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(label.to_owned()),
                );
            if !change.before.is_empty() {
                change_view = change_view.child(
                    div()
                        .text_sm()
                        .whitespace_normal()
                        .text_color(cx.theme().muted_foreground)
                        .child(change_text(lang, &change.path, &change.before)),
                );
            }
            change_view = change_view.child(
                h_flex()
                    .items_start()
                    .gap_2()
                    .child(Icon::new(IconName::ArrowRight).size_4().flex_shrink_0())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .whitespace_normal()
                            .child(change_text(lang, &change.path, &change.after)),
                    ),
            );
            card = card.child(change_view);
        }
        card = card.child(
            div()
                .text_xs()
                .whitespace_normal()
                .text_color(cx.theme().muted_foreground)
                .child(tr(
                    lang,
                    match proposal.kind.as_str() {
                        "merge" => "ai.merge_impact",
                        "dns" => "ai.dns_impact",
                        "tun" => "ai.tun_impact",
                        "system_proxy" => "ai.proxy_impact",
                        "proxy_settings" => "ai.proxy_settings_impact",
                        _ => "ai.runtime_impact",
                    },
                )),
        );
        if let Some(result) = &proposal.result {
            card = card.child(
                div()
                    .text_xs()
                    .whitespace_normal()
                    .child(result_text(lang, result)),
            );
        }
        if pending {
            let id = proposal.id.clone();
            let digest = proposal.digest.clone();
            let dismiss_id = id.clone();
            let mut actions = h_flex().gap_2().child(
                Button::new(("ai-proposal-apply", index))
                    .debug_selector(move || format!("ai-proposal-apply-{index}"))
                    .label(tr(
                        lang,
                        if confirming {
                            "ai.confirm_apply"
                        } else {
                            "ai.review_apply"
                        },
                    ))
                    .small()
                    .primary()
                    .disabled(disabled)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.ai_form.confirm_proposal.as_ref() == Some(&id) {
                            if this.try_dispatch_ai(
                                AiCommand::Approve {
                                    id: id.clone(),
                                    digest: digest.clone(),
                                },
                                cx,
                            ) {
                                this.ai_form.confirm_proposal = None;
                            }
                        } else {
                            this.ai_form.confirm_proposal = Some(id.clone());
                            cx.notify();
                        }
                    })),
            );
            actions = actions.child(
                Button::new(("ai-proposal-dismiss", index))
                    .label(tr(lang, "ai.dismiss"))
                    .small()
                    .disabled(disabled)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ai_form.confirm_proposal = None;
                        this.try_dispatch_ai(
                            AiCommand::Dismiss {
                                id: dismiss_id.clone(),
                            },
                            cx,
                        );
                    })),
            );
            if confirming {
                actions = actions.child(
                    Button::new(("ai-proposal-cancel", index))
                        .debug_selector(move || format!("ai-proposal-cancel-{index}"))
                        .label(tr(lang, "common.cancel"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.ai_form.confirm_proposal = None;
                            cx.notify();
                        })),
                );
                card = card.child(
                    div()
                        .text_sm()
                        .whitespace_normal()
                        .child(tr(lang, "ai.confirm_impact")),
                );
            }
            card = card.child(actions);
        }
        list = list.child(card);
    }
    list
}

fn result_text(lang: Lang, result: &str) -> String {
    let key = if result.starts_with("Applied and verified") {
        "ai.applied"
    } else if result.starts_with("Application failed; the previous state") {
        "ai.restored"
    } else if result.starts_with("Application and recovery failed") {
        "ai.recovery_failed"
    } else if result.starts_with("State changed or is unavailable") {
        "ai.rejected"
    } else if result.starts_with("No changes applied") {
        "ai.dismissed"
    } else {
        return result.to_owned();
    };
    let mut text = tr(lang, key).to_owned();
    if result.contains("audit result could not be saved") {
        text.push_str(tr(lang, "ai.audit_failed"));
    }
    text
}

fn change_text(lang: crate::i18n::Lang, path: &str, value: &str) -> String {
    let key = match (path, value) {
        ("proxy.use_default_bypass" | "proxy.pac_mode" | "proxy.guard_enabled", "true") => "ai.yes",
        ("proxy.use_default_bypass" | "proxy.pac_mode" | "proxy.guard_enabled", "false") => "ai.no",
        ("proxy.bypass", "") => "ai.proxy_no_custom",
        ("scope", "Apply to the currently managed system proxy and save for future use") => {
            "ai.proxy_apply_now"
        }
        ("scope", "Save preferences for the next enable; leave system proxy disabled") => {
            "ai.proxy_save_only"
        }
        _ => return value.to_owned(),
    };
    tr(lang, key).into()
}
