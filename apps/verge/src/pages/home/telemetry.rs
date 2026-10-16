//! The only dashboard entity invalidated by traffic samples. History is bounded.
mod chart;
use crate::pages::components::panel;
use crate::{
    domain::{MemoryEvent, RealtimeEvent, TrafficEvent},
    format,
    i18n::{Lang, tr},
};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, StyledExt as _, h_flex, v_flex};
use gpui_kit::{prelude::FluentBuilder as _, *};
use std::collections::VecDeque;

const HISTORY_SAMPLES: usize = 40;

pub struct Telemetry {
    history: VecDeque<TrafficEvent>,
    memory: Option<MemoryEvent>,
    connections: Option<usize>,
    totals: Option<(u64, u64)>,
    language: Lang,
}

impl Telemetry {
    pub fn new() -> Self {
        Self {
            history: VecDeque::new(),
            memory: None,
            connections: None,
            totals: None,
            language: Lang::En,
        }
    }

    pub fn set_language(&mut self, language: Lang, cx: &mut Context<Self>) {
        if self.language != language {
            self.language = language;
            cx.notify();
        }
    }

    pub fn apply(&mut self, events: &[RealtimeEvent], cx: &mut Context<Self>) {
        let mut changed = false;
        for event in events {
            match event {
                RealtimeEvent::Traffic(traffic) => {
                    if self.history.len() == HISTORY_SAMPLES {
                        self.history.pop_front();
                    }
                    self.history.push_back(traffic.clone());
                    changed = true;
                }
                RealtimeEvent::Memory(memory) => {
                    changed |= self.memory.as_ref() != Some(memory);
                    self.memory = Some(memory.clone());
                }
                RealtimeEvent::Connections(snapshot) => {
                    let totals = (snapshot.download_total, snapshot.upload_total);
                    changed |= self.connections != Some(snapshot.connection_count);
                    changed |= self.totals != Some(totals);
                    self.connections = Some(snapshot.connection_count);
                    self.totals = Some(totals);
                }
                _ => {}
            }
        }
        if changed {
            cx.notify();
        }
    }
}

impl Render for Telemetry {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = self.language;
        let up = self
            .history
            .back()
            .map_or_else(|| "—".into(), |t| format::rate(t.up));
        let down = self
            .history
            .back()
            .map_or_else(|| "—".into(), |t| format::rate(t.down));
        let download_peak = self.history.iter().map(|sample| sample.down).max();
        let upload_peak = self.history.iter().map(|sample| sample.up).max();
        let axis = chart::Axis::new(
            self.history
                .iter()
                .map(|t| t.up.max(t.down))
                .max()
                .unwrap_or(0),
        );
        let ticks = [1., 0.5, 0.].map(|fraction| axis.label(fraction));
        let tick_width = ticks.iter().fold(px(28.), |width, label| {
            let run = TextRun {
                len: label.len(),
                font: window.text_style().font(),
                ..Default::default()
            };
            width.max(
                window
                    .text_system()
                    .shape_line(label.clone().into(), px(12.), &[run], None)
                    .width()
                    .ceil()
                    + px(2.),
            )
        });
        let maximum = axis.maximum;
        let samples = self.history.clone();
        let colors = [cx.theme().chart_1, cx.theme().chart_2];
        let grid_color = cx.theme().border.opacity(0.55);
        let chart = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let bounds = chart::plot_bounds(bounds);
                let mut grid = PathBuilder::stroke(px(1.));
                for row in 0..=4 {
                    let y = bounds.top() + bounds.size.height * (row as f32 / 4.);
                    grid.move_to(point(bounds.left(), y));
                    grid.line_to(point(bounds.right(), y));
                }
                for column in 0..=8 {
                    let x = bounds.left() + bounds.size.width * (column as f32 / 8.);
                    grid.move_to(point(x, bounds.top()));
                    grid.line_to(point(x, bounds.bottom()));
                }
                if let Ok(path) = grid.build() {
                    window.paint_path(path, grid_color);
                }
                if samples.len() < 2 {
                    return;
                }
                let series: [Vec<f64>; 2] = [
                    samples.iter().map(|sample| sample.down as f64).collect(),
                    samples.iter().map(|sample| sample.up as f64).collect(),
                ];
                // Paint both fills before the strokes so neither series obscures the other.
                for (values, color) in series.iter().zip(colors) {
                    let mut area = PathBuilder::fill();
                    chart::curve(&mut area, values, bounds, maximum, HISTORY_SAMPLES, true);
                    if let Ok(path) = area.build() {
                        window.paint_path(path, color.opacity(0.08));
                    }
                }
                for (values, color) in series.iter().zip(colors) {
                    let mut line = PathBuilder::stroke(px(1.75));
                    chart::curve(&mut line, values, bounds, maximum, HISTORY_SAMPLES, false);
                    if let Ok(path) = line.build() {
                        window.paint_path(path, color);
                    }
                }
            },
        )
        .size_full();
        panel(cx)
            .debug_selector(|| "home-telemetry-panel".into())
            .min_w_0()
            .flex_1()
            .gap_3()
            .child(
                h_flex()
                    .h(px(28.))
                    .pb_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .justify_between()
                    .child(div().font_medium().child(tr(lang, "home.traffic")))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr(lang, "home.samples")),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .text_xs()
                    .line_height(px(16.))
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        div()
                            .id("traffic-scale")
                            .child(axis.unit)
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(tr(
                                    lang,
                                    "home.chart_scale",
                                ))
                                .build(window, cx)
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .child(div().text_color(colors[0]).child(tr(lang, "home.download")))
                            .child(div().text_color(colors[1]).child(tr(lang, "home.upload"))),
                    ),
            )
            .child(
                h_flex()
                    .debug_selector(|| "home-traffic-chart".into())
                    .flex_1()
                    .min_h(px(168.))
                    .gap_2()
                    .items_stretch()
                    .child(
                        v_flex()
                            .w(tick_width)
                            .flex_shrink_0()
                            .justify_between()
                            .text_xs()
                            .line_height(px(12.))
                            .text_right()
                            .text_color(cx.theme().muted_foreground)
                            .children(ticks.into_iter().enumerate().map(|(index, label)| {
                                div()
                                    .debug_selector(move || format!("home-traffic-tick-{index}"))
                                    .whitespace_nowrap()
                                    .child(label)
                            })),
                    )
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(chart)
                            .when(self.history.is_empty(), |this| {
                                this.child(
                                    div()
                                        .absolute()
                                        .inset_0()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(tr(lang, "home.traffic_waiting")),
                                )
                            }),
                    ),
            )
            .child(
                h_flex()
                    .pl(tick_width + px(8.))
                    .justify_between()
                    .text_xs()
                    .line_height(px(16.))
                    .text_color(cx.theme().muted_foreground)
                    .child(tr(lang, "home.recent"))
                    .child(tr(lang, "home.now")),
            )
            .child(
                h_flex()
                    .debug_selector(|| "home-traffic-metrics".into())
                    .gap_4()
                    .items_start()
                    .child(traffic_metric(
                        tr(lang, "home.tile.download"),
                        down,
                        self.totals.map(|(down, _)| down),
                        download_peak,
                        (colors[0], IconName::ArrowDown),
                        lang,
                        cx,
                    ))
                    .child(traffic_metric(
                        tr(lang, "home.tile.upload"),
                        up,
                        self.totals.map(|(_, up)| up),
                        upload_peak,
                        (colors[1], IconName::ArrowUp),
                        lang,
                        cx,
                    )),
            )
            .child(
                h_flex()
                    .debug_selector(|| "home-traffic-footer".into())
                    .gap_4()
                    .justify_between()
                    .text_xs()
                    .line_height(px(16.))
                    .text_color(cx.theme().muted_foreground)
                    .pt_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(format!(
                        "{} {}",
                        tr(lang, "home.core_memory"),
                        self.memory
                            .as_ref()
                            .map_or_else(|| "—".into(), |m| format::bytes(m.inuse)),
                    ))
                    .child(format!(
                        "{} {}",
                        tr(lang, "home.tile.connections"),
                        self.connections
                            .map_or_else(|| "—".into(), |n| n.to_string()),
                    )),
            )
    }
}

fn traffic_metric(
    label: &'static str,
    value: String,
    total: Option<u64>,
    peak: Option<u64>,
    style: (Hsla, IconName),
    lang: Lang,
    cx: &App,
) -> impl IntoElement {
    let (color, icon) = style;
    let detail = |label: &'static str, value: String| {
        h_flex()
            .justify_between()
            .gap_2()
            .text_xs()
            .line_height(px(16.))
            .text_color(cx.theme().muted_foreground)
            .child(label)
            .child(value)
    };
    v_flex()
        .flex_1()
        .min_w_0()
        .px_3()
        .py_2()
        .gap_1()
        .border_1()
        .border_color(cx.theme().border)
        .rounded(px(8.))
        .child(
            h_flex()
                .gap_2()
                .text_xs()
                .line_height(px(16.))
                .text_color(color)
                .child(
                    div()
                        .size(px(24.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(6.))
                        .bg(color.opacity(0.08))
                        .child(Icon::new(icon).size_4()),
                )
                .child(label),
        )
        .child(
            div()
                .debug_selector(move || format!("traffic-rate-{label}"))
                .text_2xl()
                .font_medium()
                .line_height(px(32.))
                .text_color(color)
                .child(value),
        )
        .child(
            v_flex()
                .gap_1()
                .pt_2()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(detail(
                    tr(lang, "home.core_total"),
                    total.map_or_else(|| "—".into(), format::bytes),
                ))
                .child(detail(
                    tr(lang, "home.sample_peak"),
                    peak.map_or_else(|| "—".into(), format::rate),
                )),
        )
}

#[cfg(test)]
mod tests {
    use super::{HISTORY_SAMPLES, Telemetry};
    use crate::domain::{ConnectionSnapshot, RealtimeEvent, TrafficEvent};
    use gpui_kit::{AppContext as _, TestAppContext};

    #[gpui_kit::test]
    fn long_running_traffic_keeps_only_recent_samples(cx: &mut TestAppContext) {
        let telemetry = cx.new(|_| Telemetry::new());
        telemetry.update(cx, |telemetry, cx| {
            for up in 0..10_000 {
                telemetry.apply(&[RealtimeEvent::Traffic(TrafficEvent { up, down: 0 })], cx);
            }
            assert_eq!(telemetry.history.len(), HISTORY_SAMPLES);
            assert_eq!(telemetry.history.front().unwrap().up, 9_960);
            assert_eq!(telemetry.history.back().unwrap().up, 9_999);
        });
    }

    #[gpui_kit::test]
    fn traffic_totals_update_while_connection_count_stays_the_same(cx: &mut TestAppContext) {
        use std::{cell::Cell, rc::Rc};

        let telemetry = cx.new(|_| Telemetry::new());
        let notifications = Rc::new(Cell::new(0));
        let observed = notifications.clone();
        let _subscription =
            cx.update(|cx| cx.observe(&telemetry, move |_, _| observed.set(observed.get() + 1)));
        for (index, (download_total, upload_total)) in [(1_024, 512), (8_192, 2_048), (0, 0)]
            .into_iter()
            .enumerate()
        {
            telemetry.update(cx, |telemetry, cx| {
                telemetry.apply(
                    &[RealtimeEvent::Connections(ConnectionSnapshot {
                        connection_count: 2,
                        connections: Vec::new(),
                        download_total,
                        upload_total,
                    })],
                    cx,
                );
                assert_eq!(telemetry.connections, Some(2));
                assert_eq!(telemetry.totals, Some((download_total, upload_total)));
            });
            cx.run_until_parked();
            assert_eq!(notifications.get(), index + 1);
        }
        telemetry.update(cx, |telemetry, cx| {
            telemetry.apply(
                &[RealtimeEvent::Connections(ConnectionSnapshot {
                    connection_count: 2,
                    connections: Vec::new(),
                    download_total: 0,
                    upload_total: 0,
                })],
                cx,
            );
        });
        cx.run_until_parked();
        assert_eq!(notifications.get(), 3, "unchanged counters need no redraw");
    }
}
