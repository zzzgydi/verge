use super::*;
use core::prelude::v1::test;
use std::{sync::mpsc, time::Duration};

fn snapshot(a: (u64, u64), b: (u64, u64)) -> Arc<ConnectionSnapshot> {
    Arc::new(ConnectionSnapshot {
        connections: vec![
            Connection {
                id: "a".into(),
                start: "same".into(),
                host: "a.test".into(),
                upload: a.0,
                download: a.1,
                ..Default::default()
            },
            Connection {
                id: "b".into(),
                start: "same".into(),
                host: "b.test".into(),
                upload: b.0,
                download: b.1,
                ..Default::default()
            },
        ],
        connection_count: 2,
        upload_total: a.0 + b.0,
        download_total: a.1 + b.1,
    })
}

#[gpui_kit::test]
fn connection_rates_sort_and_widths_survive_live_updates(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (tx, _rx) = mpsc::sync_channel(32);
    let (view, cx) = cx.add_window_view(|window, cx| MainView::new(tx, window, cx));
    cx.simulate_resize(size(px(1280.), px(800.)));
    let table = cx.update(|_, cx| view.read(cx).connections_table.clone());
    let now = Instant::now();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.state.page = Page::Connections;
            cx.notify();
        });
        table.update(cx, |table, cx| {
            let data = table.delegate_mut();
            data.sync_at(
                Some(snapshot((100, 500), (200, 700))),
                Default::default(),
                now,
            );
            data.sync_at(
                Some(snapshot((300, 900), (1000, 900))),
                Default::default(),
                now + Duration::from_secs(2),
            );
            assert_eq!(data.rates["a"], (100, 200));
            assert_eq!(data.rates["b"], (400, 100));
            data.perform_sort(6, ColumnSort::Descending, window, cx);
            assert_eq!(data.rows, [1, 0]);
            let same = data.snapshot.clone();
            data.sync_at(
                same,
                ConnectionFilter {
                    query: "a.test".into(),
                    ..Default::default()
                },
                now + Duration::from_secs(3),
            );
            assert_eq!(data.rows, [0]);
            assert_eq!(data.rates["a"], (100, 200));
            data.sync_at(
                Some(snapshot((2000, 1100), (1200, 1000))),
                Default::default(),
                now + Duration::from_secs(4),
            );
            assert_eq!(data.rates["a"], (850, 100));
            assert_eq!(data.rows, [0, 1]);
            data.sync_at(
                Some(snapshot((1, 2), (1, 2))),
                Default::default(),
                now + Duration::from_secs(5),
            );
            assert_eq!(data.rates["a"], (0, 0));
            data.sync_at(
                Some(snapshot((2000, 1100), (1200, 1000))),
                Default::default(),
                now + Duration::from_secs(20),
            );
            assert_eq!(data.rates["a"], (0, 0));
        });
    });
    cx.run_until_parked();
    let head = cx.debug_bounds("connection-header-1").unwrap();
    // Drag the actual header divider, then pass another realtime snapshot through MainView.
    let edge = point(head.left() + px(160. - 6.), head.center().y);
    cx.simulate_mouse_down(edge, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        edge + point(px(70.), px(0.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.simulate_mouse_move(
        edge + point(px(72.), px(0.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.simulate_mouse_up(
        edge + point(px(72.), px(0.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();
    let width = cx.update(|_, cx| table.read(cx).delegate().columns[1].width);
    assert!(
        width > px(200.),
        "drag must resize the target column: {width:?}"
    );
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.state.connections = Some(snapshot((2200, 1200), (1300, 1100)));
            view.sync_connections(cx);
        })
    });
    cx.run_until_parked();
    let resized = cx.debug_bounds("connection-header-1").unwrap();
    assert!(resized.size.width > head.size.width + px(40.));
    cx.update(|_, cx| {
        table.update(cx, |table, cx| {
            table.delegate_mut().set_language(Lang::ZhCn);
            table.refresh(cx);
            assert_eq!(table.delegate().columns[1].width, width);
        })
    });
}
