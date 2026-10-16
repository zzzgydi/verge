use super::*;
use crate::domain::ProxySnapshot;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Default, PartialEq)]
struct NodeSummary {
    low: usize,
    slow: usize,
    failed: usize,
    untested: usize,
}

impl NodeSummary {
    fn from_snapshot(
        snapshot: &ProxySnapshot,
        delays: &HashMap<String, u32>,
        errors: &HashMap<String, crate::domain::AppError>,
    ) -> Self {
        let groups: HashSet<_> = snapshot.groups.iter().map(|group| &group.name).collect();
        let mut summary = Self::default();
        for (name, node) in &snapshot.proxies {
            if node.hidden
                || groups.contains(name)
                || matches!(
                    node.kind.to_ascii_lowercase().as_str(),
                    "direct"
                        | "reject"
                        | "rejectdrop"
                        | "pass"
                        | "dns"
                        | "compatible"
                        | "selector"
                        | "urltest"
                        | "fallback"
                        | "loadbalance"
                        | "relay"
                )
            {
                continue;
            }
            let delay = if errors.contains_key(name) {
                Some(0)
            } else {
                delays.get(name).copied().or(node.delay)
            };
            match delay {
                Some(0) => summary.failed += 1,
                Some(1..200) => summary.low += 1,
                Some(_) => summary.slow += 1,
                None => summary.untested += 1,
            }
        }
        summary
    }

    fn total(&self) -> usize {
        self.low + self.slow + self.failed + self.untested
    }
}

pub(super) fn render(view: &MainView, cx: &mut Context<MainView>) -> impl IntoElement {
    let lang = view.lang();
    let summary = NodeSummary::from_snapshot(
        &view.state.proxies,
        &view.state.delays,
        &view.state.delay_errors,
    );
    let row = |key: &'static str, count: usize, color: Hsla| {
        h_flex()
            .h(px(24.))
            .gap_2()
            .text_xs()
            .child(div().size(px(6.)).rounded_full().bg(color))
            .child(div().flex_1().child(tr(lang, key)))
            .child(div().font_medium().child(count.to_string()))
    };
    panel(cx)
        .debug_selector(|| "home-nodes-panel".into())
        .gap_1()
        .child(
            h_flex()
                .h(px(24.))
                .justify_between()
                .child(div().font_medium().child(tr(lang, "home.node_latency")))
                .child(
                    Button::new("home-view-nodes")
                        .small()
                        .ghost()
                        .icon(IconName::ChevronRight)
                        .accessibility_label(tr(lang, "proxies.title"))
                        .tooltip(tr(lang, "proxies.title"))
                        .on_click(cx.listener(|this, _, _, cx| this.navigate(Page::Proxies, cx))),
                ),
        )
        .child(
            v_flex()
                .child(row("home.nodes_low", summary.low, cx.theme().success))
                .child(row("home.nodes_slow", summary.slow, cx.theme().warning))
                .child(row("home.nodes_failed", summary.failed, cx.theme().danger))
                .child(row(
                    "home.nodes_untested",
                    summary.untested,
                    cx.theme().muted_foreground,
                )),
        )
        .child(
            h_flex()
                .pt_2()
                .border_t_1()
                .border_color(cx.theme().border)
                .justify_between()
                .text_xs()
                .child(tr(lang, "home.nodes_total"))
                .child(summary.total().to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(tr(lang, "home.nodes_evidence")),
        )
}

#[cfg(test)]
mod tests {
    use super::NodeSummary;
    use crate::domain::{ProxyDetails, ProxyGroup, ProxySnapshot};
    use std::collections::HashMap;

    #[test]
    fn latency_summary_counts_unique_nodes_and_keeps_unknown_separate() {
        let mut snapshot = ProxySnapshot::default();
        for (name, kind, delay, hidden) in [
            ("fast", "Shadowsocks", Some(199), false),
            ("slow", "Vless", Some(200), false),
            ("failed", "Trojan", Some(0), false),
            ("unknown", "WireGuard", None, false),
            ("DIRECT", "Direct", Some(1), false),
            ("REJECT", "Reject", Some(0), false),
            ("hidden", "Trojan", Some(1), true),
            ("group", "Selector", Some(1), false),
        ] {
            snapshot.proxies.insert(
                name.into(),
                ProxyDetails {
                    kind: kind.into(),
                    delay,
                    hidden,
                    ..Default::default()
                },
            );
        }
        snapshot.groups.push(ProxyGroup {
            name: "group".into(),
            kind: "Selector".into(),
            selected: Some("fast".into()),
            members: vec!["fast".into(), "fast".into()],
        });
        let summary = NodeSummary::from_snapshot(&snapshot, &HashMap::new(), &HashMap::new());
        assert_eq!(
            summary,
            NodeSummary {
                low: 1,
                slow: 1,
                failed: 1,
                untested: 1
            }
        );
        assert_eq!(summary.total(), 4);
        let summary = NodeSummary::from_snapshot(
            &snapshot,
            &HashMap::from([("fast".into(), 0)]),
            &HashMap::new(),
        );
        assert_eq!(summary.low, 0);
        assert_eq!(summary.failed, 2);
        assert_eq!(
            NodeSummary::from_snapshot(&ProxySnapshot::default(), &HashMap::new(), &HashMap::new())
                .total(),
            0
        );
        let errors = HashMap::from([(
            "fast".into(),
            crate::domain::AppError::new(crate::domain::ErrorCode::RequestTimeout, "test failed"),
        )]);
        let summary =
            NodeSummary::from_snapshot(&snapshot, &HashMap::from([("fast".into(), 38)]), &errors);
        assert_eq!(
            summary.low, 0,
            "a recent failure supersedes an older successful delay"
        );
        assert_eq!(summary.failed, 2);
    }
}
