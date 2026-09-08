use super::model::Row;
use crate::{
    domain::{ProxyDetails, ProxyGroup, RuntimeCommand},
    ui::{Page, UiRequest},
};
use crate::{
    domain::{ProxySnapshot, RunMode},
    view::MainView,
};
use gpui::{AppContext as _, Focusable as _, Modifiers, TestAppContext, point, px, size};
use gpui_component::Root;
use std::{cell::RefCell, sync::mpsc};
use std::{rc::Rc, sync::Arc};

fn snapshot() -> ProxySnapshot {
    let members: Vec<String> = (0..160).map(|i| format!("Node {i:03}")).collect();
    ProxySnapshot {
        groups: vec![
            ProxyGroup {
                name: "Route".into(),
                kind: "Selector".into(),
                selected: Some("Node 140".into()),
                members: members.clone(),
            },
            ProxyGroup {
                name: "GLOBAL".into(),
                kind: "Selector".into(),
                selected: Some("Node 002".into()),
                members: members.clone(),
            },
        ],
        proxies: members
            .into_iter()
            .map(|name| {
                (
                    name,
                    ProxyDetails {
                        kind: "Shadowsocks".into(),
                        udp: Some(true),
                        ..Default::default()
                    },
                )
            })
            .collect(),
    }
}

#[gpui::test]
fn group_filter_locate_selection_and_global_layout(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (tx, rx) = mpsc::channel();
    let holder = Rc::new(RefCell::new(None));
    let copy = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        *copy.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    cx.simulate_resize(size(px(1200.), px(800.)));
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.state.page = Page::Proxies;
            view.state.mode = Some(RunMode::Rule);
            view.state.proxies = Arc::new(snapshot());
            view.sync_proxies(cx);
            cx.notify();
        })
    });
    cx.run_until_parked();
    let page = cx.update(|_, cx| view.read(cx).proxy_page.clone());
    cx.update(|_, cx| {
        assert_eq!(*page.read(cx).rows, [Row::Group(0)]);
    });
    assert!(cx.debug_bounds("proxy-group-0").is_some());
    assert!(cx.debug_bounds("proxy-group-1").is_none());
    let bounds = cx.debug_bounds("proxy-group-0").unwrap();
    cx.simulate_click(bounds.center(), Modifiers::default());
    cx.run_until_parked();
    let first = cx.debug_bounds("proxy-card-0-0").unwrap();
    let second = cx.debug_bounds("proxy-card-0-1").unwrap();
    let viewport = cx.debug_bounds("proxy-list").unwrap();
    assert!(second.right() >= viewport.right() - px(20.));
    assert!((first.size.width - second.size.width).abs() < px(1.));
    let marker = cx.debug_bounds("proxy-marker-0-0").unwrap();
    let test_button = cx.debug_bounds("delay-0-0").unwrap();
    assert!((marker.center().y - first.center().y).abs() < px(1.));
    assert!((test_button.center().y - first.center().y).abs() < px(1.));
    assert!(test_button.size.height >= px(36.));
    cx.update(|window, cx| {
        let search = page.read(cx).search.clone();
        search.read(cx).focus_handle(cx).focus(window, cx);
    });
    cx.simulate_input("Node 005");
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert_eq!(
            *page.read(cx).rows,
            [
                Row::Group(0),
                Row::Nodes {
                    group: 0,
                    members: vec![5]
                }
            ]
        )
    });
    let bounds = cx.debug_bounds("proxy-card-0-5").unwrap();
    cx.simulate_click(
        point(bounds.left() + px(50.), bounds.top() + px(20.)),
        Modifiers::default(),
    );
    cx.run_until_parked();
    assert!(rx.try_iter().any(|r|matches!(r.request,UiRequest::Runtime(RuntimeCommand::SelectProxy { group,proxy }) if group=="Route" && proxy=="Node 005")));
    let delay = cx.debug_bounds("delay-0-5").unwrap();
    cx.simulate_click(delay.center(), Modifiers::default());
    cx.run_until_parked();
    let requests: Vec<_> = rx.try_iter().collect();
    assert!(requests.iter().any(|r| matches!(&r.request, UiRequest::Runtime(RuntimeCommand::TestProxyDelay { proxy, .. }) if proxy == "Node 005")));
    assert!(!requests.iter().any(|r| matches!(
        r.request,
        UiRequest::Runtime(RuntimeCommand::SelectProxy { .. })
    )));
    // Deliver an actual failed command response through the UI reducer and retained page.
    let request = &requests
        .iter()
        .find(|r| {
            matches!(
                r.request,
                UiRequest::Runtime(RuntimeCommand::TestProxyDelay { .. })
            )
        })
        .unwrap()
        .request;
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.state.core_status = crate::ui::CoreStatus::Running;
            view.state.fail(
                request,
                crate::domain::AppError::new(
                    crate::domain::ErrorCode::RequestTimeout,
                    "test timed out",
                ),
            );
            view.sync_proxies(cx);
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert!(!page.read(cx).pending.contains("Node 005"));
        assert_eq!(
            page.read(cx).delay_errors["Node 005"].code,
            crate::domain::ErrorCode::RequestTimeout
        );
        assert!(view.read(cx).state.last_error.is_none());
    });
    // The failed card remains clickable and starts a fresh test without selecting it.
    let retry = cx.debug_bounds("delay-0-5").unwrap();
    cx.simulate_click(retry.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(rx.try_iter().any(|r| matches!(
        r.request,
        UiRequest::Runtime(RuntimeCommand::TestProxyDelay { .. })
    )));
    let locate = cx.debug_bounds("locate-0").unwrap();
    cx.simulate_click(locate.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let page = page.read(cx);
        assert!(page.query.is_empty());
        assert!(page.scroll.offset().y < px(-500.));
    });
    let node = cx
        .debug_bounds("proxy-card-0-140")
        .expect("selected node must be painted after locating");
    let selected_marker = cx.debug_bounds("proxy-marker-0-140").unwrap();
    assert_eq!(selected_marker.size, size(px(24.), px(24.)));
    assert!((selected_marker.center().y - node.center().y).abs() < px(1.));
    let list = cx.debug_bounds("proxy-list").unwrap();
    assert!(node.top() >= list.top() && node.bottom() <= list.bottom());
    let collapse = cx.debug_bounds("collapse-proxies").unwrap();
    cx.simulate_click(collapse.center(), Modifiers::default());
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(*page.read(cx).rows, [Row::Group(0)]));
    let locate = cx.debug_bounds("locate-0").unwrap();
    cx.simulate_click(locate.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("proxy-card-0-140").is_some());
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.state.mode = Some(RunMode::Global);
            view.sync_proxies(cx);
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("proxy-group-0").is_none());
    cx.simulate_resize(size(px(960.), px(640.)));
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert_eq!(page.read(cx).columns, 1);
        assert_eq!(page.read(cx).rows.len(), 160);
    });
    let node = cx.debug_bounds("proxy-card-1-0").unwrap();
    cx.simulate_click(
        point(node.left() + px(50.), node.top() + px(20.)),
        Modifiers::default(),
    );
    cx.run_until_parked();
    assert!(rx.try_iter().any(|r|matches!(r.request,UiRequest::Runtime(RuntimeCommand::SelectProxy { group,.. }) if group=="GLOBAL")));
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.state.mode = Some(RunMode::Direct);
            view.sync_proxies(cx);
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| assert!(page.read(cx).rows.is_empty()));
}
