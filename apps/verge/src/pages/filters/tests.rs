use super::{ConnectionFilter, LogFilter, RuleFilter};
use crate::{
    domain::*,
    ui::{Page, UiRequest},
    view::MainView,
};
use gpui_kit::component::Root;
use gpui_kit::{AppContext as _, Focusable as _, Modifiers, TestAppContext, px, size};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, mpsc},
};

#[test]
fn rule_search_combines_terms_with_exact_facets() {
    let rule = RuleEntry {
        kind: "DOMAIN-SUFFIX".into(),
        payload: "Example.COM".into(),
        proxy: "工作代理".into(),
        size: 0,
    };
    let mut filter = RuleFilter {
        query: " example 工作 ".into(),
        kind: Some(rule.kind.clone()),
        target: Some(rule.proxy.clone()),
    };
    assert!(filter.matches(&rule));
    filter.target = Some("DIRECT".into());
    assert!(!filter.matches(&rule));
    filter = RuleFilter {
        query: "Example file".into(),
        ..Default::default()
    };
    let provider = ProviderSummary {
        name: "example".into(),
        kind: ProviderKind::Rule,
        vehicle: "File".into(),
        updated_at: String::new(),
        item_count: 10,
    };
    assert!(filter.matches_provider(&provider));
    filter.kind = Some(rule.kind);
    assert!(!filter.matches_provider(&provider));
}

#[test]
fn connections_match_protocol_process_chain_and_hidden_fields() {
    let row = Connection {
        network: "tcp".into(),
        process: "Safari".into(),
        source: "127.0.0.1:9000".into(),
        rule_payload: "Example.COM".into(),
        chains: vec!["Hong Kong".into(), "自动选择".into()],
        ..Default::default()
    };
    let mut filter = ConnectionFilter {
        query: "9000 example".into(),
        network: Some("TCP".into()),
        process: Some("Safari".into()),
        chain: Some("自动选择".into()),
    };
    assert!(filter.matches(&row));
    filter.network = Some("UDP".into());
    assert!(!filter.matches(&row));
    let unknown = Connection::default();
    filter = ConnectionFilter {
        process: Some(String::new()),
        chain: Some("DIRECT".into()),
        ..Default::default()
    };
    assert!(filter.matches(&unknown));
    assert!(!filter.matches(&row));
}

#[test]
fn logs_combine_severity_include_and_exclude_without_changing_events() {
    let mut filter = LogFilter {
        query: "DNS timeout".into(),
        exclude: "healthcheck noise".into(),
        level: Some("problems".into()),
    };
    let row = |level: &str, payload: &str| LogEvent {
        level: level.into(),
        payload: payload.into(),
    };
    assert!(filter.matches(&row("WARN", "dns\nTIMEOUT 中文")));
    assert!(filter.matches(&row("fatal", "DNS timeout")));
    assert!(!filter.matches(&row("info", "DNS timeout")));
    assert!(!filter.matches(&row("error", "DNS timeout healthcheck")));
    assert!(!filter.matches(&row("error", "DNS timeout NOISE")));
    filter.level = Some("warning".into());
    assert!(filter.matches(&row("warn", "DNS timeout")));
    filter.query = "   ".into();
    filter.exclude.clear();
    filter.level = None;
    assert!(filter.matches(&row("trace", "anything")));
}

#[gpui_kit::test]
fn search_keeps_connection_actions_bound_to_filtered_rows_and_live_updates(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_kit::init);
    let (tx, rx) = mpsc::sync_channel(32);
    let holder = Rc::new(RefCell::new(None));
    let copy = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        *copy.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    cx.simulate_resize(size(px(960.), px(640.)));
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.state.page = Page::Connections;
            view.state.connections = Some(Arc::new(ConnectionSnapshot {
                upload_total: 0,
                download_total: 0,
                connection_count: 2,
                connections: vec![
                    Connection {
                        id: "hidden".into(),
                        host: "other.test".into(),
                        ..Default::default()
                    },
                    Connection {
                        id: "visible".into(),
                        host: "example.test".into(),
                        network: "tcp".into(),
                        process: "Safari".into(),
                        ..Default::default()
                    },
                ],
            }));
            view.sync_connections(cx);
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.read(cx)
            .filters
            .connection_search
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx)
    });
    cx.simulate_input("EXAMPLE");
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert_eq!(
            view.read(cx).connections_table.read(cx).delegate().rows,
            vec![1]
        )
    });
    let protocol = cx.debug_bounds("connections-network").unwrap();
    cx.simulate_click(protocol.center(), Modifiers::default());
    cx.run_until_parked();
    cx.simulate_keystrokes("down down enter");
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert_eq!(
            view.read(cx).filters.connections.network.as_deref(),
            Some("TCP")
        )
    });
    let close = cx.debug_bounds("close-connection-0").unwrap();
    cx.simulate_click(close.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(rx.try_iter().any(|r| matches!(r.request, UiRequest::Runtime(RuntimeCommand::CloseConnection { id }) if id == "visible")));
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            let previous = view.state.connections.as_ref().unwrap();
            let mut next = (**previous).clone();
            next.connections.reverse();
            view.state.connections = Some(Arc::new(next));
            view.sync_connections(cx);
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert_eq!(
            view.read(cx).connections_table.read(cx).delegate().rows,
            vec![0]
        )
    });
    let clear = cx.debug_bounds("clear-connections-filter").unwrap();
    let toolbar = cx.debug_bounds("connections-filters").unwrap();
    assert!(clear.right() <= toolbar.right() + px(1.));
    cx.simulate_click(clear.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let v = view.read(cx);
        assert!(v.filters.connections.query.is_empty());
        assert_eq!(v.connections_table.read(cx).delegate().rows.len(), 2);
    });
}

#[gpui_kit::test]
fn rule_and_log_inputs_filter_real_rows_and_clear_all_conditions(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (tx, _rx) = mpsc::sync_channel(32);
    let holder = Rc::new(RefCell::new(None));
    let copy = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        *copy.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    cx.simulate_resize(size(px(960.), px(640.)));
    cx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.state.page = Page::Rules;
            v.state.rules = ["example.test", "other.test"]
                .into_iter()
                .map(|payload| RuleEntry {
                    kind: "DomainSuffix".into(),
                    payload: payload.into(),
                    proxy: "DIRECT".into(),
                    size: 0,
                })
                .collect();
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.read(cx)
            .filters
            .rule_search
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx)
    });
    cx.simulate_input("EXAMPLE");
    cx.run_until_parked();
    assert!(cx.debug_bounds("rule-row-0").is_some());
    assert!(cx.debug_bounds("rule-row-1").is_none());
    cx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.state.page = Page::Logs;
            for payload in ["DNS timeout", "DNS healthcheck timeout", "TCP connected"] {
                v.state.apply_realtime(RealtimeEvent::Log(LogEvent {
                    level: "warning".into(),
                    payload: payload.into(),
                }));
            }
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.read(cx)
            .filters
            .log_search
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx)
    });
    cx.simulate_input("DNS");
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.read(cx)
            .filters
            .log_exclude
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx)
    });
    cx.simulate_input("healthcheck");
    cx.run_until_parked();
    assert!(cx.debug_bounds("log-row-0").is_some());
    assert!(cx.debug_bounds("log-row-1").is_none());
    assert!(cx.debug_bounds("log-row-2").is_none());
    let clear = cx.debug_bounds("clear-log-filter").unwrap();
    cx.simulate_click(clear.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("log-row-1").is_some());
    assert!(cx.debug_bounds("log-row-2").is_some());
    cx.update(|_, cx| {
        view.update(cx, |v, cx| {
            v.state.page = Page::Rules;
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("rule-row-1").is_none());
}
