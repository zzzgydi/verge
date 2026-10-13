mod presentation;
mod sections;
#[cfg(test)]
mod tests;
use crate::{
    i18n::{self, Lang, tr},
    optimization::{Command, Phase, Report, Severity, Snapshot},
    ui::{Page, UiAction},
    view::MainView,
};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    StyledExt as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement as _,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::*;
use presentation::{Next, finding, nodes, summarize};

#[derive(Clone, Copy, Default, PartialEq)]
enum Tab {
    #[default]
    Result,
    History,
    Settings,
}

pub struct Panel {
    snapshot: Snapshot,
    lang: Lang,
    disconnected: bool,
    pending: bool,
    pub view: Option<WeakEntity<MainView>>,
    pub optimize: bool,
    selected_group: Option<String>,
    tab: Tab,
    details: bool,
    consent: bool,
    probe_consent: bool,
    selected_history: Option<u64>,
    result_scroll: ScrollHandle,
    history_scroll: ScrollHandle,
    settings_scroll: ScrollHandle,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            snapshot: Snapshot::default(),
            lang: Lang::En,
            disconnected: false,
            pending: false,
            view: None,
            optimize: false,
            selected_group: None,
            tab: Tab::Result,
            details: false,
            consent: false,
            probe_consent: false,
            selected_history: None,
            result_scroll: ScrollHandle::new(),
            history_scroll: ScrollHandle::new(),
            settings_scroll: ScrollHandle::new(),
        }
    }
}

impl Panel {
    pub fn sync(
        &mut self,
        snapshot: Snapshot,
        lang: Lang,
        disconnected: bool,
        pending: bool,
        cx: &mut Context<Self>,
    ) {
        if self.snapshot == snapshot
            && self.lang == lang
            && self.disconnected == disconnected
            && self.pending == pending
        {
            return;
        }
        if self.snapshot.report.as_ref().map(|r| r.id) != snapshot.report.as_ref().map(|r| r.id) {
            self.consent = false;
            self.details = false;
            self.selected_group = None;
        }
        if snapshot.active_probes {
            self.probe_consent = false;
        }
        if let Some(report) = &snapshot.report
            && self
                .selected_group
                .as_ref()
                .is_none_or(|name| !report.groups.contains(name))
        {
            self.selected_group = None;
        }
        self.snapshot = snapshot;
        self.lang = lang;
        self.disconnected = disconnected;
        self.pending = pending;
        cx.notify();
    }

    fn disabled(&self) -> bool {
        self.pending || self.snapshot.busy() || self.disconnected
    }

    fn send(&mut self, command: Command, cx: &mut Context<Self>) {
        if self.disconnected || self.pending {
            return;
        }
        if !matches!(command, Command::Cancel) && self.snapshot.busy() {
            return;
        }
        if let Some(view) = self.view.clone() {
            self.pending = true;
            // Dispatch after this entity update so MainView can safely sync the panel.
            cx.defer(move |cx| {
                let _ = view.update(cx, |view, cx| {
                    view.dispatch(UiAction::Optimization(command), cx);
                    let pending = view.is_pending(&["optimization_write"]);
                    view.optimization_panel.update(cx, |panel, cx| {
                        panel.pending = pending;
                        panel.disconnected = view.state.connection_notice.is_some();
                        cx.notify();
                    });
                });
            });
            cx.notify();
        }
    }

    fn request_check(&mut self, cx: &mut Context<Self>) {
        self.tab = Tab::Result;
        self.result_scroll.set_offset(point(px(0.), px(0.)));
        if self.snapshot.active_probes {
            self.send(Command::Check, cx);
        } else {
            self.probe_consent = true;
            cx.notify();
        }
    }

    fn navigate(&self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        window.close_sheet(cx);
        if let Some(view) = self.view.clone() {
            cx.defer(move |cx| {
                let _ = view.update(cx, |view, cx| {
                    if page == Page::Settings {
                        view.settings_category = crate::pages::settings::SettingsCategory::Network;
                    }
                    view.navigate(page, cx);
                });
            });
        }
    }
}

pub fn status(snapshot: &Snapshot) -> &'static str {
    summarize(snapshot).title
}

impl MainView {
    pub(crate) fn open_optimization(
        &mut self,
        optimize: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel = self.optimization_panel.clone();
        let view = cx.entity().downgrade();
        let lang = self.lang();
        let pending = self.is_pending(&["optimization_write"]);
        panel.update(cx, |panel, cx| {
            panel.view = Some(view);
            panel.optimize = optimize;
            panel.selected_group = None;
            panel.tab = Tab::Result;
            panel.probe_consent = false;
            panel.result_scroll.set_offset(point(px(0.), px(0.)));
            panel.sync(
                self.state.optimization.clone(),
                lang,
                self.state.connection_notice.is_some(),
                pending,
                cx,
            );
        });
        window.open_sheet(cx, move |sheet, _, _| {
            sheet
                .title(tr(lang, "health.title"))
                .size(px(600.))
                .child(panel.clone())
        });
    }
}

fn color(severity: Severity, cx: &App) -> Hsla {
    match severity {
        Severity::Pass => cx.theme().success,
        Severity::Warning => cx.theme().danger,
        Severity::Notice => cx.theme().primary,
        Severity::Unknown => cx.theme().muted_foreground,
    }
}
fn mark(severity: Severity) -> IconName {
    match severity {
        Severity::Pass => IconName::Check,
        Severity::Warning => IconName::Info,
        _ => IconName::Info,
    }
}
fn note(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .line_height(px(18.))
        .text_color(cx.theme().muted_foreground)
        .whitespace_normal()
        .child(text.into())
}
fn time_label(timestamp: u64) -> String {
    chrono::DateTime::from_timestamp(timestamp as i64, 0)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

impl Render for Panel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = self.lang();
        let mut tabs = h_flex()
            .gap_1()
            .px_4()
            .pb_2()
            .border_b_1()
            .border_color(cx.theme().border);
        for (index, (tab, label)) in [
            (Tab::Result, "health.tab_result"),
            (Tab::History, "health.tab_history"),
            (Tab::Settings, "health.tab_settings"),
        ]
        .into_iter()
        .enumerate()
        {
            tabs = tabs.child(
                Button::new(("health-tab", index))
                    .debug_selector(move || format!("health-tab-{index}"))
                    .label(tr(lang, label))
                    .small()
                    .ghost()
                    .selected(self.tab == tab)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab = tab;
                        cx.notify();
                    })),
            );
        }
        let mut root = v_flex().flex_1().min_h_0().child(tabs);
        if self.disconnected {
            root = root.child(
                div()
                    .px_4()
                    .py_2()
                    .text_color(cx.theme().danger)
                    .child(tr(lang, "health.disconnected")),
            );
        }
        if let Some(error) = &self.snapshot.error {
            root = root.child(
                div()
                    .debug_selector(|| "health-operation-error".into())
                    .px_4()
                    .py_2()
                    .text_sm()
                    .whitespace_normal()
                    .text_color(cx.theme().danger)
                    .child(message(lang, error)),
            );
        }
        if self.tab == Tab::Result {
            root = root.child(self.header(cx));
        }
        let body = match self.tab {
            Tab::Result => self.results(cx),
            Tab::Settings => self.settings(cx),
            Tab::History => self.history(cx),
        };
        let scroll = match self.tab {
            Tab::Result => &self.result_scroll,
            Tab::History => &self.history_scroll,
            Tab::Settings => &self.settings_scroll,
        };
        root.child(
            v_flex()
                .flex_1()
                .min_h_0()
                .id(match self.tab {
                    Tab::Result => "health-results",
                    Tab::Settings => "health-settings",
                    Tab::History => "health-history",
                })
                .overflow_y_scroll()
                .track_scroll(scroll)
                .vertical_scrollbar(scroll)
                .child(body),
        )
    }
}

impl Panel {
    fn lang(&self) -> Lang {
        self.lang
    }

    fn header(&self, cx: &mut Context<Self>) -> Div {
        let lang = self.lang;
        let verdict = summarize(&self.snapshot);
        let pending_start = self.pending && !self.snapshot.busy();
        let busy = self.snapshot.busy() || self.pending;
        let accent = color(verdict.severity, cx);
        let mut header =
            v_flex()
                .gap_3()
                .p_4()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    h_flex()
                        .gap_3()
                        .items_start()
                        .child(
                            div()
                                .size_10()
                                .flex_shrink_0()
                                .rounded_full()
                                .bg(accent.opacity(0.10))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    Icon::new(if busy {
                                        IconName::LoaderCircle
                                    } else {
                                        mark(verdict.severity)
                                    })
                                    .size_5()
                                    .text_color(accent),
                                ),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                .child(div().text_lg().font_semibold().whitespace_normal().child(
                                    tr(
                                        lang,
                                        if pending_start {
                                            "health.starting"
                                        } else {
                                            verdict.title
                                        },
                                    ),
                                ))
                                .child(note(
                                    tr(
                                        lang,
                                        if pending_start {
                                            "health.starting_detail"
                                        } else {
                                            verdict.detail
                                        },
                                    ),
                                    cx,
                                )),
                        ),
                );
        let mut actions = h_flex().gap_2().flex_wrap();
        if busy {
            actions = actions.child(
                Button::new("health-busy")
                    .label(tr(lang, "health.working"))
                    .small()
                    .loading(true)
                    .disabled(true),
            );
            if self.snapshot.phase != Phase::Verifying {
                actions = actions.child(
                    Button::new("health-cancel")
                        .debug_selector(|| "health-cancel".into())
                        .label(tr(lang, "common.cancel"))
                        .small()
                        .ghost()
                        .disabled(self.pending || self.disconnected)
                        .on_click(cx.listener(|this, _, _, cx| this.send(Command::Cancel, cx))),
                );
            }
        } else {
            let label = match verdict.next {
                Next::Profiles => "health.open_profiles",
                Next::Network => "health.open_network",
                Next::Proxies => "health.open_proxies",
                Next::Logs => "health.open_logs",
                Next::Compare => "health.compare",
                Next::Review => "health.review_change",
                Next::Probe => "health.test_nodes",
                _ => "health.check_again",
            };
            let next = verdict.next;
            actions = actions.child(
                Button::new("health-primary")
                    .debug_selector(|| "health-primary".into())
                    .label(tr(lang, label))
                    .small()
                    .primary()
                    .disabled(self.disabled())
                    .on_click(cx.listener(move |this, _, window, cx| match next {
                        Next::Profiles => this.navigate(Page::Profiles, window, cx),
                        Next::Network => this.navigate(Page::Settings, window, cx),
                        Next::Proxies => this.navigate(Page::Proxies, window, cx),
                        Next::Logs => this.navigate(Page::Logs, window, cx),
                        Next::Check => this.send(Command::Check, cx),
                        Next::Compare | Next::Review => {
                            this.optimize = true;
                            if next == Next::Compare {
                                this.selected_group = None;
                            }
                            this.result_scroll.set_offset(point(px(0.), px(0.)));
                            cx.notify();
                        }
                        _ => this.request_check(cx),
                    })),
            );
            if !matches!(next, Next::Check | Next::None | Next::Probe) {
                actions = actions.child(
                    Button::new("health-run")
                        .label(tr(lang, "health.check_again"))
                        .small()
                        .ghost()
                        .disabled(self.disabled())
                        .on_click(cx.listener(|this, _, _, cx| this.request_check(cx))),
                );
            }
            if let Some(report) = &self.snapshot.report {
                actions = actions.child(note(time_label(report.captured_at), cx));
            }
        }
        header = header.child(actions);
        if self.snapshot.busy()
            && let Some(report) = &self.snapshot.report
        {
            let local = report
                .findings
                .iter()
                .filter(|f| ["profile", "core", "system_proxy"].contains(&f.code.as_str()))
                .count()
                .to_string();
            let probes = report.measurements.len().to_string();
            header = header.child(note(
                i18n::fmt(
                    lang,
                    "health.progress",
                    &[("local", &local), ("probes", &probes)],
                ),
                cx,
            ));
        }
        header
    }

    fn results(&self, cx: &mut Context<Self>) -> Div {
        let lang = self.lang;
        let mut body = v_flex().gap_4().p_4().min_w_0();
        if self.probe_consent {
            body = body.child(self.probe_prompt(cx));
        }
        let Some(report) = &self.snapshot.report else {
            return body.child(note(tr(lang, "health.awaiting"), cx));
        };
        if self.optimize || report.plan.is_some() {
            body = body.child(self.comparison(report, cx));
        }
        body = body.child(self.check_rows(report, cx));
        if !report.measurements.is_empty() {
            body = body.child(self.node_results(report, cx));
        }
        if let Some(outcome) = &report.outcome {
            body = body.child(note(message(lang, outcome), cx));
        }
        body = body.child(
            Button::new("health-details")
                .debug_selector(|| "health-details".into())
                .label(tr(lang, "health.evidence"))
                .icon(if self.details {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .small()
                .ghost()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.details = !this.details;
                    cx.notify();
                })),
        );
        if self.details {
            body = body.child(self.evidence(report, cx));
        }
        body.child(self.ai(report, cx))
    }

    fn probe_prompt(&self, cx: &mut Context<Self>) -> Div {
        v_flex()
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().primary.opacity(0.4))
            .bg(cx.theme().primary.opacity(0.04))
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child(tr(self.lang, "health.probe_title")),
            )
            .child(note(tr(self.lang, "health.probe_consent"), cx))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("health-active-probes")
                            .debug_selector(|| "health-active-probes".into())
                            .label(tr(self.lang, "health.allow_check"))
                            .small()
                            .primary()
                            .disabled(self.disabled())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.probe_consent = false;
                                this.send(Command::AllowProbesAndCheck, cx);
                            })),
                    )
                    .child(
                        Button::new("health-local-only")
                            .debug_selector(|| "health-local-only".into())
                            .label(tr(self.lang, "health.only_local"))
                            .small()
                            .ghost()
                            .disabled(self.disabled())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.probe_consent = false;
                                this.send(Command::Check, cx);
                            })),
                    ),
            )
    }

    fn check_rows(&self, report: &Report, cx: &App) -> Div {
        let mut rows = v_flex().gap_1().child(
            div()
                .text_xs()
                .font_semibold()
                .text_color(cx.theme().muted_foreground)
                .child(tr(self.lang, "health.local_checks")),
        );
        for (code, label) in [
            ("profile", "optimization.profile"),
            ("core", "optimization.core"),
            ("system_proxy", "optimization.system_proxy"),
        ] {
            let state = finding(report, code).map(|f| f.severity);
            let row_label = if state.is_none() && !self.snapshot.busy() {
                "health.unread"
            } else {
                presentation::row_text(code, state)
            };
            rows = rows.child(
                h_flex()
                    .gap_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border.opacity(0.6))
                    .child(
                        Icon::new(mark(state.unwrap_or(Severity::Unknown)))
                            .size_4()
                            .text_color(color(state.unwrap_or(Severity::Unknown), cx)),
                    )
                    .child(div().flex_1().text_sm().child(tr(self.lang, label)))
                    .child(note(tr(self.lang, row_label), cx)),
            );
        }
        rows
    }

    fn node_results(&self, report: &Report, cx: &App) -> Div {
        let rows = nodes(report);
        let count = rows.len().to_string();
        let mut body = v_flex().gap_2().child(
            h_flex()
                .justify_between()
                .child(div().text_sm().font_semibold().child(i18n::fmt(
                    self.lang,
                    "health.nodes_count",
                    &[("count", &count)],
                )))
                .child(note(tr(self.lang, "health.latest_probe"), cx)),
        );
        body = body.child(
            h_flex().gap_2().child(div().flex_1()).children(
                ["Google", "Cloudflare"]
                    .map(|label| div().w(px(90.)).text_right().child(note(label, cx))),
            ),
        );
        for (index, (name, samples)) in rows.into_iter().enumerate() {
            let full_name = name.to_owned();
            let mut row = h_flex()
                .gap_2()
                .py_2()
                .border_b_1()
                .border_color(cx.theme().border.opacity(0.6))
                .child(
                    div()
                        .id(("health-node-name", index))
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .truncate()
                        .tooltip(move |window, cx| {
                            Tooltip::new(full_name.clone()).build(window, cx)
                        })
                        .child(name.to_owned()),
                );
            for sample in samples {
                let (label, severity) = match sample {
                    Some(m) => match m.delay_ms {
                        Some(ms) if ms > 0 => (format!("{ms} ms"), Severity::Pass),
                        _ => (
                            tr(self.lang, "health.unreachable").into(),
                            Severity::Warning,
                        ),
                    },
                    None => (tr(self.lang, "health.untested").into(), Severity::Unknown),
                };
                row = row.child(
                    div()
                        .w(px(90.))
                        .flex_shrink_0()
                        .text_right()
                        .text_sm()
                        .text_color(color(severity, cx))
                        .child(label),
                );
            }
            body = body.child(row);
        }
        body.child(note(
            tr(
                self.lang,
                if report.partial {
                    "health.node_partial"
                } else {
                    "health.node_scope"
                },
            ),
            cx,
        ))
    }
}
fn message(lang: Lang, code: &str) -> String {
    let key = match code {
        "optimization.probes_required" => "optimization.probes_required",
        "optimization.no_change" => "optimization.no_change",
        "optimization.no_candidate" => "optimization.no_candidate",
        "optimization.inconclusive" => "optimization.inconclusive",
        "optimization.not_applied" => "optimization.not_applied",
        "optimization.applied" => "optimization.applied",
        "optimization.restored" => "optimization.restored",
        "optimization.expired" => "optimization.expired",
        "optimization.stale" => "optimization.stale",
        "optimization.verifying" => "optimization.verifying",
        "optimization.busy" => "optimization.busy",
        "optimization.group_unavailable" => "optimization.group_unavailable",
        "optimization.storage_failed" => "optimization.storage_failed",
        "optimization.model_failed" => "optimization.model_failed",
        "optimization.network_unknown" => "optimization.network_unknown",
        "optimization.apply_unknown" => "optimization.apply_unknown",
        "optimization.recovery_failed" => "optimization.recovery_failed",
        _ => "optimization.failed",
    };
    tr(lang, key).into()
}
