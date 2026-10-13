use super::{Panel, Tab};
use crate::{
    i18n::Lang,
    optimization::{Command, Finding, Measurement, Report, Severity, Snapshot},
    ui::{Page, UiRequest, UiResponse, UiResponseEnvelope},
    view::MainView,
};
use gpui_kit::component::Root;
use gpui_kit::{AppContext as _, ElementId, Modifiers, TestAppContext, px, size};
use std::{cell::RefCell, rc::Rc, sync::mpsc};

#[gpui_kit::test]
fn comparison_does_not_guess_a_group_or_reuse_another_reports_choice(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let (tx, rx) = mpsc::sync_channel(32);
    let holder = Rc::new(RefCell::new(None));
    let slot = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        view.update(cx, |v, cx| {
            v.state.page = Page::Home;
            v.state
                .daemon_capabilities
                .push(crate::optimization::CAPABILITY.into());
            v.state.optimization = Snapshot {
                active_probes: true,
                report: Some(Report {
                    id: 1,
                    groups: vec!["A-working".into(), "B-failed".into()],
                    measurements: vec![Measurement {
                        node: "B-node".into(),
                        target: 0,
                        delay_ms: None,
                        captured_at: 1,
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            };
            v.sync_form_inputs(window, cx);
            *slot.borrow_mut() = Some(v.optimization_panel.clone());
        });
        let root = Root::new(view.clone(), window, cx);
        window.defer(cx, move |w, cx| {
            view.update(cx, |v, cx| v.open_optimization(false, w, cx))
        });
        root
    });
    let panel = holder.borrow().clone().unwrap();
    cx.simulate_resize(size(px(960.), px(640.)));
    cx.run_until_parked();
    let primary = cx.debug_bounds("health-primary").unwrap();
    cx.simulate_click(primary.center(), Modifiers::default());
    cx.run_until_parked();
    let compare = cx.debug_bounds("health-optimize").unwrap();
    cx.simulate_click(compare.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(
        rx.try_recv().is_err(),
        "no unrelated default group may be tested"
    );
    let picker = cx.debug_bounds("health-group-picker").unwrap();
    cx.simulate_click(picker.center(), Modifiers::default());
    cx.run_until_parked();
    let choice = cx.update(|window, _| {
        gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .find(|e| e.visible() && e.label() == Some("B-failed"))
            .expect("group choice is visible")
            .bounds()
    });
    cx.simulate_click(choice.center(), Modifiers::default());
    cx.run_until_parked();
    let compare = cx.debug_bounds("health-optimize").unwrap();
    cx.simulate_click(compare.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(
        matches!(rx.try_recv().unwrap().request, UiRequest::Optimization(Command::Optimize { group }) if group == "B-failed")
    );
    cx.update(|_, cx| {
        panel.update(cx, |p, cx| {
            let mut next = p.snapshot.clone();
            next.report.as_mut().unwrap().id = 2;
            p.sync(next, Lang::En, false, false, cx);
            assert!(
                p.selected_group.is_none(),
                "new evidence must not reuse a previous choice"
            );
        })
    });
}

#[gpui_kit::test]
fn failed_preferences_show_errors_and_a_sole_report_can_be_cleared(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (tx, rx) = mpsc::sync_channel(32);
    let holder = Rc::new(RefCell::new(None));
    let slot = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        view.update(cx, |v, _| {
            v.state
                .daemon_capabilities
                .push(crate::optimization::CAPABILITY.into());
            v.state.optimization.report = Some(Report {
                id: 1,
                ..Default::default()
            });
        });
        *slot.borrow_mut() = Some(view.clone());
        let root = Root::new(view.clone(), window, cx);
        window.defer(cx, move |w, cx| {
            view.update(cx, |v, cx| v.open_optimization(false, w, cx))
        });
        root
    });
    let view = holder.borrow().clone().unwrap();
    cx.run_until_parked();
    let tab = cx.debug_bounds("health-tab-2").unwrap();
    cx.simulate_click(tab.center(), Modifiers::default());
    cx.run_until_parked();
    let enable = cx.update(|window, _| {
        gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .find(|e| e.visible() && e.path().contains(&ElementId::from("health-observe")))
            .unwrap()
            .bounds()
    });
    cx.simulate_click(enable.center(), Modifiers::default());
    cx.run_until_parked();
    let request = rx.try_recv().unwrap();
    assert!(matches!(
        request.request,
        UiRequest::Optimization(Command::Configure { observe: true, .. })
    ));
    cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.state
                .apply_response_envelope(UiResponseEnvelope::for_request(
                    &request,
                    UiResponse::Optimization {
                        command: Command::Configure {
                            observe: true,
                            model_analysis: false,
                        },
                        result: Err(crate::optimization::error("optimization.storage_failed")),
                    },
                ));
            v.sync_form_inputs(window, cx);
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("health-operation-error").is_some());
    assert!(!cx.update(|_, cx| view.read(cx).state.optimization.observe));
    let tab = cx.debug_bounds("health-tab-1").unwrap();
    cx.simulate_click(tab.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("health-operation-error").is_some());
    let clear = cx.debug_bounds("health-clear").unwrap();
    cx.simulate_click(clear.center(), Modifiers::default());
    cx.run_until_parked();
    let clear_request = rx.try_recv().unwrap();
    assert!(matches!(
        clear_request.request,
        UiRequest::Optimization(Command::ClearHistory)
    ));
    cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.state
                .apply_response_envelope(UiResponseEnvelope::for_request(
                    &clear_request,
                    UiResponse::Optimization {
                        command: Command::ClearHistory,
                        result: Ok(Snapshot {
                            revision: 10,
                            ..Default::default()
                        }),
                    },
                ));
            v.sync_form_inputs(window, cx);
        });
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("health-clear").is_none());
    assert!(cx.debug_bounds("health-operation-error").is_none());
}

#[gpui_kit::test]
fn historical_terminal_failure_keeps_the_incomplete_verdict(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let report = Report {
        findings: vec![Finding {
            code: "check_failed".into(),
            severity: Severity::Unknown,
            detail: String::new(),
        }],
        measurements: (0..2)
            .map(|target| Measurement {
                node: "a".into(),
                target,
                delay_ms: Some(20),
                captured_at: 1,
            })
            .collect(),
        ..Default::default()
    };
    let (_, cx) = cx.add_window_view(|window, cx| {
        let panel = cx.new(|_| Panel {
            tab: Tab::History,
            snapshot: Snapshot {
                history: vec![report],
                ..Default::default()
            },
            ..Default::default()
        });
        Root::new(panel, window, cx)
    });
    cx.run_until_parked();
    assert!(cx.update(|window, _| {
        gpui_kit::base::test_support::snapshots(window)
            .iter()
            .any(|e| {
                e.label().is_some_and(|label| {
                    label.contains(crate::i18n::tr(Lang::En, "health.incomplete"))
                })
            })
    }));
}
