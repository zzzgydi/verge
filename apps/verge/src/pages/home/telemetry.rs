//! The only dashboard entity invalidated by traffic samples. History is bounded.
use crate::pages::components::{Metric, panel};
use crate::{
    domain::{MemoryEvent, RealtimeEvent, TrafficEvent},
    format,
    i18n::{Lang, tr},
};
use gpui::{prelude::FluentBuilder as _, *};
use gpui_component::{ActiveTheme as _, IconName, StyledExt as _, h_flex, v_flex};
use std::collections::VecDeque;

const HISTORY_SAMPLES: usize = 40;

pub struct Telemetry {
    history: VecDeque<TrafficEvent>,
    memory: Option<MemoryEvent>,
    connections: Option<usize>,
    language: Lang,
}

impl Telemetry {
    pub fn new() -> Self {
        Self {
            history: VecDeque::new(),
            memory: None,
            connections: None,
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
                    changed |= self.connections != Some(snapshot.connection_count);
                    self.connections = Some(snapshot.connection_count);
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
        let bar = |value: u64, color: Hsla| {
            div()
                .flex_1()
                .h(px((value as f64 / peak * 86.) as f32))
                .rounded_t(px(2.))
                .bg(color)
        };
        let bars = (0..HISTORY_SAMPLES).map(|ix| {
            let sample = ix
                .checked_sub(HISTORY_SAMPLES - self.history.len())
                .and_then(|ix| self.history.get(ix));
            h_flex()
                .flex_1()
                .h_full()
                .items_end()
                .gap(px(2.))
                .child(bar(sample.map_or(0, |t| t.down), cx.theme().foreground))
                .child(bar(sample.map_or(0, |t| t.up), cx.theme().muted_foreground))
        });
        v_flex()
            .gap_4()
            .child(
                panel(cx)
                    .gap_3()
                    .child(
                        h_flex()
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
                            .gap_6()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(tr(lang, "home.tile.download")),
                                    )
                                    .child(div().text_2xl().child(down)),
                            )
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(tr(lang, "home.tile.upload")),
                                    )
                                    .child(
                                        div()
                                            .text_2xl()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(up),
                                    ),
                            ),
                    )
                    .child(
                        v_flex()
                            .relative()
                            .h(px(92.))
                            .justify_end()
                            .child(
                                h_flex()
                                    .w_full()
                                    .h_full()
                                    .items_end()
                                    .gap(px(4.))
                                    .children(bars),
                            )
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
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .pt_3()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr(lang, "home.recent"))
                            .child(tr(lang, "home.now")),
                    ),
            )
            .child(
                h_flex()
                    .gap_4()
                    .child(Metric::new(
                        tr(lang, "home.core_memory"),
                        self.memory
                            .as_ref()
                            .map_or_else(|| "—".into(), |m| format::bytes(m.inuse)),
                        IconName::Cpu,
                    ))
                    .child(Metric::new(
                        tr(lang, "home.tile.connections"),
                        self.connections
                            .map_or_else(|| "—".into(), |n| n.to_string()),
                        IconName::Network,
                    )),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{HISTORY_SAMPLES, Telemetry};
    use crate::domain::{RealtimeEvent, TrafficEvent};
    use gpui::{AppContext as _, TestAppContext};

    #[gpui::test]
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
}
