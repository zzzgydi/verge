use core::prelude::v1::test;
use gpui::*;
use gpui_component::Root;
use gpui_component::scroll::ScrollbarHandle as _;
use std::{cell::RefCell, rc::Rc, sync::mpsc};

use crate::{
    domain::{LogEvent, RealtimeEvent},
    ui::Page,
    view::MainView,
};

#[gpui::test]
fn multiline_logs_fit_and_longest_log_scrolls_both_axes(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (tx, _rx) = mpsc::channel();
    let holder = Rc::new(RefCell::new(None));
    let copy = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        *copy.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.simulate_resize(size(px(960.), px(640.)));
    let view = holder.borrow().clone().unwrap();
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.state.apply_realtime(RealtimeEvent::Log(LogEvent {
                level: "debug".into(),
                payload: "short first line".into(),
            }));
            view.state.apply_realtime(RealtimeEvent::Log(LogEvent {
                level: "warning".into(),
                payload: "[Update padding failed]\r\nsecond response\n\n🇬🇧 节点详情\tEND".into(),
            }));
            // The longest line isn't the first or initially visible item.
            for i in 2..120 {
                view.state.apply_realtime(RealtimeEvent::Log(LogEvent {
                    level: "debug".into(),
                    payload: if i == 100 {
                        format!("{} END-OF-LONG-LOG", "long payload 中文 / ".repeat(100))
                    } else {
                        format!("message {i}")
                    },
                }));
            }
            view.navigate(Page::Logs, cx);
        })
    });
    cx.run_until_parked();
    let row = cx.debug_bounds("log-row-1").unwrap();
    let text = cx.debug_bounds("log-text-1").unwrap();
    let next = cx.debug_bounds("log-row-2").unwrap();
    assert_eq!(row.size.height, px(96.));
    assert!(
        text.top() >= row.top() && text.bottom() <= row.bottom(),
        "{text:?} vs {row:?}"
    );
    assert_eq!(row.bottom(), next.top());
    let viewport = cx.debug_bounds("log-viewport").unwrap();
    let scroll = cx.update(|_, cx| view.read(cx).log_scroll.clone());
    assert!(scroll.content_size().width > viewport.size.width * 4.);
    let before = cx.debug_bounds("log-row-0").unwrap();
    cx.simulate_event(ScrollWheelEvent {
        position: viewport.center(),
        delta: ScrollDelta::Pixels(point(px(-320.), px(0.))),
        modifiers: Default::default(),
        touch_phase: TouchPhase::Moved,
    });
    cx.run_until_parked();
    assert!(scroll.offset().x < px(-100.));
    assert!(cx.debug_bounds("log-row-0").unwrap().left() < before.left() - px(100.));
    cx.simulate_event(ScrollWheelEvent {
        position: viewport.center(),
        delta: ScrollDelta::Pixels(point(px(0.), px(-350.))),
        modifiers: Default::default(),
        touch_phase: TouchPhase::Moved,
    });
    cx.run_until_parked();
    assert!(scroll.offset().y < px(-100.));
    cx.update(|_, cx| {
        view.update(cx, |_, cx| {
            scroll.scroll_to_item(100, ScrollStrategy::Top);
            scroll.set_offset(point(px(-1_000_000.), scroll.offset().y));
            cx.notify();
        })
    });
    cx.run_until_parked();
    let longest = cx.debug_bounds("log-row-100").unwrap();
    assert!(
        (longest.right() - scroll.bounds().right()).abs() <= px(1.),
        "end of longest line must be reachable"
    );
    let offset = scroll.offset();
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.state.apply_realtime(RealtimeEvent::Log(LogEvent {
                level: "info".into(),
                payload: "new event while scrolled".into(),
            }));
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert_eq!(scroll.offset(), offset);
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.log_filter = Some("warning");
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert_eq!(scroll.offset(), point(px(0.), px(0.)));
    assert!(scroll.content_size().width < viewport.size.width);
    assert!(cx.debug_bounds("log-row-1").is_some());
    // Buffer eviction must discard cached widths instead of keeping an old outlier.
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.log_filter = None;
            for _ in 0..510 {
                view.state.apply_realtime(RealtimeEvent::Log(LogEvent {
                    level: "info".into(),
                    payload: "short replacement".into(),
                }));
            }
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert!(scroll.content_size().width < viewport.size.width);
    assert_eq!(cx.debug_bounds("log-row-0").unwrap().size.height, px(24.));
}
