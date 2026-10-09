//! The only dashboard entity invalidated by traffic samples. History is bounded.
use crate::pages::components::{Metric, panel};
use crate::{
    domain::{MemoryEvent, RealtimeEvent, TrafficEvent},
    format,
    i18n::{Lang, tr},
};
use gpui_kit::component::{ActiveTheme as _, StyledExt as _, h_flex, v_flex};
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lang = self.language;
        let up = self
            .history
            .back()
            .map_or_else(|| "—".into(), |t| format::rate(t.up));
        let down = self
            .history
            .back()
            .map_or_else(|| "—".into(), |t| format::rate(t.down));
        let peak = self
            .history
            .iter()
            .map(|t| t.up.max(t.down))
            .max()
            .unwrap_or(0)
            .max(1) as f64;
        let samples = self.history.clone();
        let colors = [cx.theme().foreground, cx.theme().muted_foreground];
        let chart = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                // A fixed history mapped to the actual plot width keeps strokes readable
                // in the minimum-size window; missing samples remain empty on the left.
                for (series, color) in colors.into_iter().enumerate() {
                    let mut path = PathBuilder::stroke(px(2.));
                    for (index, sample) in samples.iter().enumerate() {
                        let x = bounds.left()
                            + bounds.size.width
                                * ((HISTORY_SAMPLES - samples.len() + index) as f32
                                    / (HISTORY_SAMPLES - 1) as f32);
                        let value = if series == 0 { sample.down } else { sample.up };
                        let y = bounds.bottom()
                            - px(2.)
                            - (bounds.size.height - px(4.)) * (value as f64 / peak) as f32;
                        if index == 0 {
                            path.move_to(point(x, y));
                        } else {
                            path.line_to(point(x, y));
                        }
                    }
                    if samples.len() >= 2
                        && let Ok(path) = path.build()
                    {
                        window.paint_path(path, color);
                    }
                }
            },
        )
        .size_full();
        panel(cx)
            .debug_selector(|| "home-telemetry-panel".into())
            .min_w_0()
            .gap_2()
            .child(
                h_flex()
                    .h(px(28.))
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
                    .gap_4()
                    .items_start()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .line_height(px(16.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(tr(lang, "home.tile.download")),
                            )
                            .child(div().text_2xl().line_height(px(28.)).child(down))
                            .child(
                                div()
                                    .text_xs()
                                    .line_height(px(16.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{} {}",
                                        tr(lang, "home.core_total"),
                                        self.totals.map_or_else(
                                            || "—".into(),
                                            |(down, _)| format::bytes(down)
                                        ),
                                    )),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .line_height(px(16.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(tr(lang, "home.tile.upload")),
                            )
                            .child(
                                div()
                                    .text_2xl()
                                    .line_height(px(28.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(up),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .line_height(px(16.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{} {}",
                                        tr(lang, "home.core_total"),
                                        self.totals.map_or_else(
                                            || "—".into(),
                                            |(_, up)| format::bytes(up)
                                        ),
                                    )),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .relative()
                    .flex_1()
                    .min_h(px(48.))
                    .justify_end()
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
            )
            .child(
                h_flex()
                    .justify_between()
                    .text_xs()
                    .line_height(px(16.))
                    .text_color(cx.theme().muted_foreground)
                    .child(tr(lang, "home.recent"))
                    .child(tr(lang, "home.now")),
            )
            .child(
                h_flex()
                    .gap_4()
                    .pt_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(Metric::new(
                        tr(lang, "home.core_memory"),
                        self.memory
                            .as_ref()
                            .map_or_else(|| "—".into(), |m| format::bytes(m.inuse)),
                    ))
                    .child(Metric::new(
                        tr(lang, "home.tile.connections"),
                        self.connections
                            .map_or_else(|| "—".into(), |n| n.to_string()),
                    )),
            )
    }
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
