use super::*;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};

impl Panel {
    pub(super) fn comparison(&self, report: &Report, cx: &mut Context<Self>) -> Div {
        let lang = self.lang;
        let mut body = v_flex()
            .gap_2()
            .p_3()
            .rounded_lg()
            .bg(cx.theme().muted)
            .child(div().text_sm().font_semibold().child(tr(
                lang,
                if report.plan.is_some() {
                    "health.change_proposal"
                } else {
                    "health.compare"
                },
            )));
        if let Some(plan) = &report.plan {
            let id = plan.id.clone();
            return body
                .child(note(plan.group.clone(), cx))
                .child(
                    h_flex()
                        .gap_2()
                        .min_w_0()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .child(plan.previous.clone()),
                        )
                        .child(Icon::new(IconName::ArrowRight).size_4())
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .font_semibold()
                                .child(plan.proposed.clone()),
                        ),
                )
                .child(note(tr(lang, "health.switch_consent"), cx))
                .child(
                    Button::new("health-apply")
                        .debug_selector(|| "health-apply".into())
                        .label(tr(lang, "optimization.confirm"))
                        .small()
                        .primary()
                        .disabled(
                            self.disabled()
                                || !self.snapshot.active_probes
                                || plan.expires_at <= crate::optimization::now(),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.send(Command::Apply { id: id.clone() }, cx)
                        })),
                );
        }
        if report.groups.is_empty() {
            return body
                .child(note(tr(lang, "health.no_manual_groups"), cx))
                .child(
                    Button::new("health-view-proxies")
                        .label(tr(lang, "health.open_proxies"))
                        .small()
                        .ghost()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.navigate(Page::Proxies, window, cx)
                        })),
                );
        }
        if !self.snapshot.active_probes {
            return body
                .child(note(tr(lang, "health.compare_needs_probe"), cx))
                .child(
                    Button::new("health-compare-probe")
                        .label(tr(lang, "health.test_nodes"))
                        .small()
                        .disabled(self.disabled())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.request_check(cx);
                        })),
                );
        }
        body = body.child(note(tr(lang, "health.compare_scope"), cx));
        let groups = report.groups.clone();
        let owner = cx.entity().downgrade();
        body = body.child(
            Button::new("health-group-picker")
                .debug_selector(|| "health-group-picker".into())
                .label(
                    self.selected_group
                        .clone()
                        .unwrap_or_else(|| tr(lang, "health.choose_group").into()),
                )
                .icon(IconName::ChevronDown)
                .small()
                .disabled(self.disabled())
                .dropdown_menu(move |mut menu, _, _| {
                    for group in &groups {
                        let group = group.clone();
                        let owner = owner.clone();
                        menu = menu.item(PopupMenuItem::new(group.clone()).on_click(
                            move |_, _, cx| {
                                let _ = owner.update(cx, |panel, cx| {
                                    panel.selected_group = Some(group.clone());
                                    cx.notify();
                                });
                            },
                        ));
                    }
                    menu
                }),
        );
        let group = self.selected_group.clone();
        body.child(
            Button::new("health-optimize")
                .debug_selector(|| "health-optimize".into())
                .label(tr(lang, "health.find_replacement"))
                .small()
                .primary()
                .disabled(
                    self.disabled() || group.as_ref().is_none_or(|g| !report.groups.contains(g)),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(group) = &group {
                        this.send(
                            Command::Optimize {
                                group: group.clone(),
                            },
                            cx,
                        );
                    }
                })),
        )
    }

    pub(super) fn evidence(&self, report: &Report, cx: &App) -> Div {
        let mut body = v_flex()
            .gap_2()
            .pl_3()
            .border_l_2()
            .border_color(cx.theme().border);
        for f in &report.findings {
            let label = match f.code.as_str() {
                "profile" => "optimization.profile",
                "core" => "optimization.core",
                "system_proxy" => "optimization.system_proxy",
                "tun" => "optimization.tun",
                "dns" => "optimization.dns",
                "logs" => "optimization.logs",
                "connections" => "optimization.connections",
                "probes_disabled" => "health.untested",
                "direct_mode" => "health.direct",
                "check_failed" => "health.incomplete",
                _ => "health.evidence",
            };
            let status = match f.severity {
                Severity::Pass => "optimization.pass",
                Severity::Notice => "optimization.notice",
                Severity::Warning => "optimization.warning",
                Severity::Unknown => "health.untested",
            };
            body = body.child(
                h_flex()
                    .gap_2()
                    .justify_between()
                    .child(note(tr(self.lang, label), cx))
                    .child(note(format!("{} {}", tr(self.lang, status), f.detail), cx)),
            );
        }
        body = body
            .child(note(tr(self.lang, "health.evidence_scope"), cx))
            .child(note(crate::optimization::PROBES.join("\n"), cx));
        for m in &report.measurements {
            body = body.child(note(
                format!(
                    "{} / {} / {}",
                    m.node,
                    if m.target == 0 {
                        "Google"
                    } else {
                        "Cloudflare"
                    },
                    m.delay_ms.map(|d| format!("{d} ms")).unwrap_or_else(|| tr(
                        self.lang,
                        "health.unreachable"
                    )
                    .into())
                ),
                cx,
            ));
        }
        body
    }

    pub(super) fn ai(&self, report: &Report, cx: &mut Context<Self>) -> Div {
        let mut body = v_flex()
            .gap_2()
            .pt_2()
            .border_t_1()
            .border_color(cx.theme().border);
        if let Some(text) = &report.explanation {
            body = body
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .child(tr(self.lang, "health.ai_explanation")),
                )
                .child(
                    div()
                        .text_sm()
                        .line_height(px(22.))
                        .whitespace_normal()
                        .child(text.clone()),
                );
        }
        body = body.child(
            Button::new("health-ai-consent")
                .label(tr(self.lang, "health.ask_ai"))
                .small()
                .ghost()
                .disabled(self.disabled())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.consent = !this.consent;
                    cx.notify();
                })),
        );
        if self.consent {
            let id = report.id;
            body = body
                .child(note(tr(self.lang, "health.ai_consent"), cx))
                .child(
                    Button::new("health-ai-run")
                        .label(tr(self.lang, "optimization.send"))
                        .small()
                        .disabled(self.disabled())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.consent = false;
                            this.send(Command::Analyze { report_id: id }, cx);
                        })),
                );
        }
        body
    }

    pub(super) fn settings(&self, cx: &mut Context<Self>) -> Div {
        let lang = self.lang;
        let mut body = v_flex().gap_4().p_4().child(
            div()
                .text_sm()
                .font_semibold()
                .child(tr(lang, "health.settings_title")),
        );
        let active = self.snapshot.active_probes;
        body = body.child(
            v_flex()
                .gap_2()
                .child(
                    h_flex()
                        .gap_2()
                        .justify_between()
                        .child(div().text_sm().child(tr(lang, "health.active_probes")))
                        .child(
                            Button::new("health-probe-setting")
                                .label(tr(
                                    lang,
                                    if active {
                                        "health.turn_off"
                                    } else {
                                        "health.turn_on"
                                    },
                                ))
                                .small()
                                .disabled(self.disabled())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if active {
                                        this.send(Command::SetActiveProbes { enabled: false }, cx);
                                    } else {
                                        this.request_check(cx);
                                    }
                                })),
                        ),
                )
                .child(note(tr(lang, "health.probe_setting_detail"), cx)),
        );
        let observe = self.snapshot.observe;
        body = body.child(
            v_flex()
                .gap_2()
                .pt_3()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(
                    h_flex()
                        .gap_2()
                        .justify_between()
                        .child(div().text_sm().child(tr(lang, "health.observation")))
                        .child(
                            Button::new("health-observe")
                                .label(tr(
                                    lang,
                                    if observe {
                                        "health.turn_off"
                                    } else {
                                        "health.turn_on"
                                    },
                                ))
                                .small()
                                .disabled(self.disabled())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.send(
                                        Command::Configure {
                                            observe: !observe,
                                            model_analysis: false,
                                        },
                                        cx,
                                    )
                                })),
                        ),
                )
                .child(note(tr(lang, "health.observation_detail"), cx)),
        );
        if observe {
            let model = self.snapshot.model_analysis;
            body = body.child(
                v_flex()
                    .gap_2()
                    .pt_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .gap_2()
                            .justify_between()
                            .child(div().text_sm().child(tr(lang, "health.auto_ai")))
                            .child(
                                Button::new("health-background-model")
                                    .label(tr(
                                        lang,
                                        if model {
                                            "health.turn_off"
                                        } else {
                                            "health.turn_on"
                                        },
                                    ))
                                    .small()
                                    .disabled(self.disabled())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.send(
                                            Command::Configure {
                                                observe: true,
                                                model_analysis: !model,
                                            },
                                            cx,
                                        )
                                    })),
                            ),
                    )
                    .child(note(tr(lang, "health.auto_ai_detail"), cx))
                    .child(note(tr(lang, "health.ai_consent"), cx)),
            );
        }
        body.child(note(tr(lang, "health.retention"), cx))
    }

    pub(super) fn history(&self, cx: &mut Context<Self>) -> Div {
        let mut body = v_flex().gap_3().p_4();
        if self.snapshot.history.is_empty() {
            body = body.child(note(tr(self.lang, "health.no_history"), cx));
        }
        for (index, r) in self.snapshot.history.iter().enumerate() {
            let verdict = summarize(&Snapshot {
                report: Some(r.clone()),
                ..Default::default()
            });
            let id = r.id;
            let open = self.selected_history == Some(id);
            body = body.child(
                Button::new(("health-history-row", index))
                    .label(format!(
                        "{}   {}",
                        time_label(r.captured_at),
                        tr(self.lang, verdict.title)
                    ))
                    .icon(if open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .small()
                    .ghost()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected_history = if open { None } else { Some(id) };
                        cx.notify();
                    })),
            );
            if open {
                body = body
                    .child(note(tr(self.lang, "health.historical"), cx))
                    .child(self.check_rows(r, cx));
                if !r.measurements.is_empty() {
                    body = body.child(self.node_results(r, cx));
                }
            }
        }
        if self.snapshot.report.is_none() && self.snapshot.history.is_empty() {
            return body;
        }
        body.child(
            Button::new("health-clear")
                .debug_selector(|| "health-clear".into())
                .label(tr(self.lang, "optimization.clear_history"))
                .small()
                .ghost()
                .disabled(self.disabled())
                .on_click(cx.listener(|this, _, _, cx| this.send(Command::ClearHistory, cx))),
        )
    }
}
