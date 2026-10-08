use super::*;

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
            .gap_2()
            .min_w_0()
            .child(div().text_sm().font_semibold().child(format!(
                "{} · {}",
                tr(lang, "ai.proposal"),
                proposal.target
            )))
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
            card = card.child(div().text_sm().whitespace_normal().child(format!(
                "{}\n{} → {}",
                match change.path.as_str() {
                    "Added Merge operations" => tr(lang, "ai.merge_operations"),
                    "scope" => tr(lang, "ai.scope"),
                    _ => &change.path,
                },
                change.before,
                change.after
            )));
        }
        card = card.child(
            div()
                .text_xs()
                .whitespace_normal()
                .text_color(cx.theme().muted_foreground)
                .child(tr(
                    lang,
                    if proposal.kind == "merge" {
                        "ai.merge_impact"
                    } else {
                        "ai.runtime_impact"
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
