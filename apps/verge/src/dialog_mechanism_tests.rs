//! Root / Dialog / Sheet 机制的 UI 测试。
//!
//! 用 gpui 的 `add_window_view`（与 GPUI Kit 官方测试一致的环境）验证：
//! 1. dialog/sheet 打开后由 Root 自动挂载到画面，不由 MainView 手动渲染浮层；
//! 2. sheet builder 渲染时不能读 MainView 实体（已用 SheetState 解耦）。

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use gpui_kit::component::{ActiveTheme as _, Root, WindowExt as _};
use gpui_kit::{AppContext as _, Styled as _, TestAppContext, VisualTestContext};

use crate::i18n::Lang;
use crate::view::MainView;

type ViewHolder = Rc<RefCell<Option<gpui_kit::Entity<MainView>>>>;

fn sheet_content_is_visible(window: &gpui_kit::Window) -> bool {
    let sheet_content = gpui_kit::ElementId::from("sheet-content");
    gpui_kit::base::test_support::snapshots(window)
        .iter()
        .any(|element| element.visible() && element.path().contains(&sheet_content))
}

#[gpui_kit::test]
fn system_proxy_switch_shows_pending_without_claiming_success(cx: &mut TestAppContext) {
    use crate::{
        domain::*,
        ui::{Page, UiRequest, UiResponse, UiResponseEnvelope},
    };
    use gpui_kit::{Modifiers, px, size};
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let (tx, rx) = mpsc::sync_channel(32);
    let holder: ViewHolder = Default::default();
    let slot = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        view.update(cx, |v, _| {
            v.state.application_settings = Some(ApplicationSettingsSnapshot {
                settings: ApplicationSettings::default(),
                data_directory: "/tmp/test".into(),
                app_version: None,
            });
            v.state.runtime_settings = Some(RuntimeSettings {
                system_proxy_services: vec!["Wi-Fi".into()],
                system_proxy_endpoint: ProxyEndpoint::new("127.0.0.1", 7897).unwrap(),
                system_proxy_socks_endpoint: None,
            });
        });
        *slot.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    cx.simulate_resize(size(px(960.), px(640.)));
    for page in [Page::Home, Page::Settings] {
        cx.update(|_, cx| {
            view.update(cx, |v, cx| {
                v.state.page = page;
                v.settings_category = crate::pages::settings::SettingsCategory::Network;
                cx.notify();
            })
        });
        cx.run_until_parked();
        let id = if page == Page::Home {
            "system-proxy"
        } else {
            "settings-system-proxy-toggle"
        };
        let toggle = cx.update(|window, _| {
            gpui_kit::base::test_support::snapshots(window)
                .into_iter()
                .find(|element| {
                    element.visible() && element.path().contains(&gpui_kit::ElementId::from(id))
                })
                .expect("system proxy switch is visible")
                .bounds()
        });
        cx.simulate_click(toggle.center(), Modifiers::default());
        cx.run_until_parked();
        let request = rx.try_recv().unwrap();
        assert_eq!(
            request.request,
            UiRequest::SystemProxy(SystemProxyCommand::SetEnabled { enabled: true })
        );
        cx.update(|_, cx| {
            view.update(cx, |v, _| {
                assert!(v.state.system_proxy_switching());
                assert!(!v.state.system_proxy_enabled());
            })
        });
        let initial_bounds = toggle;
        let toggle = cx.update(|window, _| {
            gpui_kit::base::test_support::snapshots(window)
                .into_iter()
                .find(|element| {
                    element.visible() && element.path().contains(&gpui_kit::ElementId::from(id))
                })
                .expect("system proxy switch is visible")
                .bounds()
        });
        assert_eq!(
            toggle, initial_bounds,
            "pending text must not move the switch"
        );
        if page == Page::Home {
            let pending = cx.debug_bounds("home-proxy-switching").unwrap();
            assert!(
                pending.left() >= toggle.right(),
                "pending text belongs to the right of the switch"
            );
        }
        cx.simulate_click(toggle.center(), Modifiers::default());
        cx.run_until_parked();
        assert!(
            rx.try_recv().is_err(),
            "pending switch cannot enqueue another mutation"
        );
        cx.update(|_, cx| {
            view.update(cx, |v, cx| {
                v.state
                    .apply_response_envelope(UiResponseEnvelope::for_request(
                        &request,
                        UiResponse::SystemProxy {
                            request: SystemProxyCommand::SetEnabled { enabled: true },
                            result: Err(AppError::new(
                                ErrorCode::PlatformFailed,
                                "simulated failure",
                            )),
                        },
                    ));
                assert!(!v.state.system_proxy_switching());
                assert!(!v.state.system_proxy_enabled());
                assert!(v.state.last_error.is_some());
                // Compare the switch layout without the separate page-level error banner.
                v.state.clear_error();
                cx.notify();
            })
        });
        cx.run_until_parked();
        let restored = cx.update(|window, _| {
            gpui_kit::base::test_support::snapshots(window)
                .into_iter()
                .find(|element| {
                    element.visible() && element.path().contains(&gpui_kit::ElementId::from(id))
                })
                .unwrap()
                .bounds()
        });
        assert_eq!(
            restored, initial_bounds,
            "removing pending text must not move the switch"
        );
    }
}

#[gpui_kit::test]
fn optimization_overview_buttons_and_sheet_work_in_both_themes(cx: &mut TestAppContext) {
    use crate::{
        optimization::{Command, Phase, Report, Snapshot},
        ui::{Page, UiRequest, UiResponse},
    };
    use gpui_kit::component::theme::{Theme, ThemeMode};
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.set_reduce_motion(true);
    });
    let (tx, rx) = mpsc::sync_channel(32);
    let holder: ViewHolder = Default::default();
    let slot = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        view.update(cx, |view, _| {
            view.state.page = Page::Home;
            view.state
                .daemon_capabilities
                .push(crate::optimization::CAPABILITY.into());
        });
        *slot.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for (width, height) in [(960., 640.), (1280., 800.)] {
            cx.simulate_resize(gpui_kit::size(gpui_kit::px(width), gpui_kit::px(height)));
            cx.update(|window, cx| {
                Theme::change(mode, Some(window), cx);
                view.update(cx, |view, cx| {
                    view.state.pending.clear();
                    view.state.optimization = Snapshot::default();
                    view.sync_form_inputs(window, cx);
                    cx.notify();
                });
                window.close_sheet(cx);
            });
            cx.run_until_parked();
            let check = cx.debug_bounds("home-health-check").unwrap();
            let optimize = cx.debug_bounds("home-health-optimize").unwrap();
            assert!(check.bottom() < gpui_kit::px(height));
            assert!(optimize.right() < gpui_kit::px(width));
            cx.simulate_click(check.center(), gpui_kit::Modifiers::default());
            cx.run_until_parked();
            let check_request = rx.try_recv().unwrap();
            assert!(matches!(
                check_request.request,
                UiRequest::Optimization(Command::Check)
            ));
            assert!(cx.update(|window, _| sheet_content_is_visible(window)));
            cx.update(|window, cx| {
                view.update(cx, |view, cx| {
                    view.state
                        .apply_response_envelope(crate::ui::UiResponseEnvelope::for_request(
                            &check_request,
                            UiResponse::Optimization {
                                command: Command::Check,
                                result: Ok(Snapshot {
                                    revision: 2,
                                    phase: Phase::Checking,
                                    report: Some(Report {
                                        id: 1,
                                        partial: true,
                                        ..Default::default()
                                    }),
                                    ..Default::default()
                                }),
                            },
                        ));
                    view.sync_form_inputs(window, cx);
                    cx.notify();
                })
            });
            cx.run_until_parked();
            let cancel = cx.debug_bounds("health-cancel").unwrap();
            assert!(cancel.bottom() < gpui_kit::px(height));
            cx.simulate_click(cancel.center(), gpui_kit::Modifiers::default());
            cx.run_until_parked();
            let cancel_request = rx.try_recv().unwrap();
            assert!(matches!(
                cancel_request.request,
                UiRequest::Optimization(Command::Cancel)
            ));
            assert!(rx.try_recv().is_err());
            cx.update(|_, cx| {
                view.update(cx, |view, _| {
                    view.state
                        .apply_response_envelope(crate::ui::UiResponseEnvelope::for_request(
                            &cancel_request,
                            UiResponse::Optimization {
                                command: Command::Cancel,
                                result: Ok(Snapshot {
                                    revision: 3,
                                    ..Default::default()
                                }),
                            },
                        ));
                })
            });
        }
    }
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.state.pending.clear();
            view.state.optimization = Snapshot {
                revision: 20,
                report: Some(Report {
                    id: 20,
                    findings: vec![crate::optimization::Finding {
                        code: "probes_disabled".into(),
                        severity: crate::optimization::Severity::Unknown,
                        detail: String::new(),
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            };
            view.sync_form_inputs(window, cx);
            view.open_optimization(false, window, cx);
        });
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("health-active-probes").is_none(),
        "consent is not a permanent wall"
    );
    let primary = cx.debug_bounds("health-primary").unwrap();
    cx.simulate_click(primary.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(
        rx.try_recv().is_err(),
        "opening consent cannot grant permission"
    );
    let probes = cx.debug_bounds("health-active-probes").unwrap();
    cx.simulate_click(probes.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(matches!(
        rx.try_recv().unwrap().request,
        UiRequest::Optimization(Command::AllowProbesAndCheck)
    ));
    assert!(
        rx.try_recv().is_err(),
        "one click dispatches one atomic request"
    );
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(1280.), gpui_kit::px(960.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.state.pending.clear();
            view.state.optimization = Snapshot {
                revision: 30,
                active_probes: true,
                report: Some(Report {
                    id: 30,
                    captured_at: crate::optimization::now(),
                    plan: Some(crate::optimization::Plan {
                        id: "bound-confirmation".into(),
                        group: "Manual".into(),
                        previous: "Old node".into(),
                        proposed: "New node".into(),
                        expires_at: crate::optimization::now() + 120,
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            };
            view.sync_form_inputs(window, cx);
            view.open_optimization(true, window, cx);
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert!(rx.try_recv().is_err(), "showing a proposal cannot apply it");
    let apply = cx.debug_bounds("health-apply").unwrap();
    assert!(apply.bottom() < gpui_kit::px(960.));
    cx.simulate_click(apply.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(
        matches!(rx.try_recv().unwrap().request, UiRequest::Optimization(Command::Apply { id }) if id == "bound-confirmation")
    );
    assert!(rx.try_recv().is_err());
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(960.), gpui_kit::px(640.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.state.pending.clear();
            view.state.optimization = Snapshot {
                revision: 35,
                active_probes: true,
                report: Some(Report {
                    id: 35,
                    groups: vec!["Manual".into()],
                    measurements: (0..20)
                        .map(|index| crate::optimization::Measurement {
                            node: format!("Long node name {index}"),
                            target: 0,
                            delay_ms: None,
                            captured_at: crate::optimization::now(),
                        })
                        .collect(),
                    ..Default::default()
                }),
                ..Default::default()
            };
            view.sync_form_inputs(window, cx);
            view.open_optimization(false, window, cx);
        });
    });
    cx.run_until_parked();
    let primary = cx.debug_bounds("health-primary").unwrap();
    cx.simulate_click(primary.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(rx.try_recv().is_err(), "comparing starts only on request");
    let compare = cx.debug_bounds("health-optimize").unwrap();
    assert!(compare.top() > primary.bottom());
    assert!(
        compare.bottom() < gpui_kit::px(640.),
        "next action must appear above the node list"
    );
    cx.simulate_click(compare.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(
        rx.try_recv().is_err(),
        "comparison requires an explicit group choice"
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.state.pending.clear();
            view.state.optimization = Snapshot {
                revision: 40,
                report: Some(Report {
                    id: 40,
                    findings: vec![crate::optimization::Finding {
                        code: "profile".into(),
                        severity: crate::optimization::Severity::Unknown,
                        detail: String::new(),
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            };
            view.sync_form_inputs(window, cx);
            view.open_optimization(false, window, cx);
        });
    });
    cx.run_until_parked();
    let settings = cx.debug_bounds("health-tab-2").unwrap();
    cx.simulate_click(settings.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("health-primary").is_none(),
        "preferences are not mixed into results"
    );
    let results = cx.debug_bounds("health-tab-0").unwrap();
    cx.simulate_click(results.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    let primary = cx.debug_bounds("health-primary").unwrap();
    cx.simulate_click(primary.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert!(!sheet_content_is_visible(window));
        view.read_with(cx, |view, _| assert_eq!(view.state.page, Page::Profiles));
    });
}

#[gpui_kit::test]
fn ai_requests_are_gated_and_reconnect_does_not_restart_a_turn(cx: &mut TestAppContext) {
    use crate::{
        ai::{AiCommand, AiOperation, AiSnapshot, ChatMessage},
        ui::{Page, UiAction, UiRequest, UiResponse},
    };
    cx.update(gpui_kit::init);
    let (tx, rx) = mpsc::sync_channel(32);
    let holder: ViewHolder = Default::default();
    let slot = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        *slot.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.navigate(Page::Ai, cx);
            assert!(rx.try_recv().is_err());
            view.state
                .daemon_capabilities
                .push(crate::ai::CAPABILITY.into());
            view.navigate(Page::Ai, cx);
            assert!(matches!(
                rx.try_recv().unwrap().request,
                UiRequest::Ai(AiCommand::GetState)
            ));
            view.ai_form.prompt.update(cx, |input, cx| {
                input.set_value("draft question", window, cx)
            });
            let snapshot = AiSnapshot {
                revision: 10,
                busy: true,
                messages: vec![ChatMessage {
                    reasoning: String::new(),
                    evidence: Vec::new(),
                    role: "assistant".into(),
                    text: "partial".into(),
                }],
                ..Default::default()
            };
            view.state.apply_response(UiResponse::Ai {
                operation: AiOperation::State,
                result: Ok(snapshot),
            });
            view.state.apply_response(UiResponse::Ai {
                operation: AiOperation::State,
                result: Ok(AiSnapshot {
                    revision: 9,
                    ..Default::default()
                }),
            });
            assert_eq!(view.state.ai.messages[0].text, "partial");
            view.daemon_disconnected(window, cx);
            view.dispatch(
                UiAction::Ai(AiCommand::Start {
                    prompt: "ignored while offline".into(),
                }),
                cx,
            );
            assert!(rx.try_recv().is_err());
            view.state.connection_notice = None;
            view.refresh_current_page(cx);
            assert!(matches!(
                rx.try_recv().unwrap().request,
                UiRequest::Ai(AiCommand::GetState)
            ));
            assert!(rx.try_recv().is_err());
            assert_eq!(
                view.ai_form.prompt.read(cx).value().as_str(),
                "draft question"
            );
        })
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn ai_send_and_stop_remain_visible_and_dispatch_from_buttons(cx: &mut TestAppContext) {
    use crate::{
        ai::{AiCommand, ProviderConfig},
        ui::{Page, UiRequest},
    };
    cx.update(gpui_kit::init);
    let (tx, rx) = mpsc::sync_channel(32);
    let holder: ViewHolder = Default::default();
    let slot = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        *slot.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(960.), gpui_kit::px(640.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.state.page = Page::Ai;
            view.state
                .daemon_capabilities
                .push(crate::ai::CAPABILITY.into());
            view.state.ai.config = ProviderConfig {
                base_url: "https://example.test/v1".into(),
                model: "fake".into(),
                system_prompt: String::new(),
            };
            view.state.ai.revision = 1;
            view.ai_form.prompt.update(cx, |input, cx| {
                input.set_value("为什么无法联网？", window, cx)
            });
            cx.notify();
        })
    });
    cx.run_until_parked();
    let send = cx.debug_bounds("ai-send").unwrap();
    assert!(send.bottom() < gpui_kit::px(640.));
    cx.simulate_click(send.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(
        matches!(rx.try_recv().unwrap().request,UiRequest::Ai(AiCommand::Start {prompt}) if prompt=="为什么无法联网？")
    );
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.state.ai.busy = true;
            cx.notify();
        })
    });
    cx.run_until_parked();
    let stop = cx.debug_bounds("ai-stop").unwrap();
    assert!(stop.bottom() < gpui_kit::px(640.));
    cx.simulate_click(stop.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(matches!(
        rx.try_recv().unwrap().request,
        UiRequest::Ai(AiCommand::Cancel)
    ));
}

/// 创建窗口：MainView + Root（与真实 main() 同构）。
fn setup(cx: &mut TestAppContext) -> (gpui_kit::Entity<Root>, ViewHolder, &mut VisualTestContext) {
    cx.update(gpui_kit::init);
    let (request_tx, _request_rx) = mpsc::sync_channel(32);
    let view_holder: ViewHolder = Default::default();
    let holder = view_holder.clone();
    let (root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(request_tx, window, cx));
        *holder.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx).bg(cx.theme().background)
    });
    (root, view_holder, cx)
}

#[gpui_kit::test]
fn import_dialog_opens_and_renders(cx: &mut TestAppContext) {
    let (_, view_holder, cx) = setup(cx);
    let view = view_holder
        .borrow()
        .clone()
        .expect("view should be created");

    // 直接调用按钮回调路径（VisualTestContext 上下文，不经 WindowHandle::update，
    // 避免无谓的 root lease）。
    cx.update(|window, cx| {
        view.update(cx, |view, cx| view.open_import_dialog(window, cx));
    });
    cx.run_until_parked();

    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
    assert!(
        cx.debug_bounds("dialog-layer").is_some(),
        "dialog must reach the Root overlay"
    );
}

#[gpui_kit::test]
fn yaml_sheet_opens_and_renders(cx: &mut TestAppContext) {
    let (_, view_holder, cx) = setup(cx);
    let view = view_holder
        .borrow()
        .clone()
        .expect("view should be created");

    let id = crate::domain::ProfileId::parse("demo").unwrap();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| view.open_yaml_sheet(id, window, cx));
    });
    cx.run_until_parked();

    assert!(cx.update(|window, cx| window.has_active_sheet(cx)));
    assert!(
        cx.update(|window, _| sheet_content_is_visible(window)),
        "sheet must reach the Root overlay"
    );
}

#[gpui_kit::test]
fn merge_sheet_opens_and_renders_without_reading_main_view(cx: &mut TestAppContext) {
    let (_, view_holder, cx) = setup(cx);
    let view = view_holder
        .borrow()
        .clone()
        .expect("view should be created");

    cx.update(|window, cx| {
        view.update(cx, |view, cx| view.open_merge_sheet(window, cx));
    });
    // 渲染（sheet builder 执行）；此前 builder 读 MainView 会 double-lease panic，
    // 现在状态在 SheetState 实体里，渲染必须无 panic 且渲染层存在。
    cx.run_until_parked();

    assert!(cx.update(|window, cx| window.has_active_sheet(cx)));
    assert!(
        cx.update(|window, _| sheet_content_is_visible(window)),
        "merge sheet must reach the Root overlay"
    );
}

#[gpui_kit::test]
fn merged_sheet_opens_and_renders(cx: &mut TestAppContext) {
    let (_, view_holder, cx) = setup(cx);
    let view = view_holder
        .borrow()
        .clone()
        .expect("view should be created");

    let id = crate::domain::ProfileId::parse("demo").unwrap();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| view.open_merged_sheet(id, window, cx));
    });
    cx.run_until_parked();

    assert!(cx.update(|window, cx| window.has_active_sheet(cx)));
    assert!(
        cx.update(|window, _| sheet_content_is_visible(window)),
        "merged sheet must reach the Root overlay"
    );
}

/// 语言切换：设置写入后 MainView::lang 立即反映新语言，sync_form_inputs
/// 同步重设输入框占位（状态驱动，无需重启；占位内容正确性由 i18n 查表测试覆盖）。
#[gpui_kit::test]
fn language_switch_updates_view_language(cx: &mut TestAppContext) {
    use crate::domain::{ApplicationSettings, ApplicationSettingsSnapshot};

    let (_, view_holder, cx) = setup(cx);
    let view = view_holder
        .borrow()
        .clone()
        .expect("view should be created");

    // 设置未加载时回退英文（与 domain 默认语言一致）。
    cx.update(|_, cx| {
        view.update(cx, |view, _| assert_eq!(view.lang(), Lang::En));
    });

    // 写入 zh-CN 设置并同步表单：语言随即切换。
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.state.application_settings = Some(ApplicationSettingsSnapshot {
                settings: ApplicationSettings {
                    language: "zh-CN".into(),
                    ..Default::default()
                },
                data_directory: "/tmp/verge-i18n-test".into(),
                app_version: None,
            });
            view.sync_form_inputs(window, cx);
            assert_eq!(view.lang(), Lang::ZhCn);
        });
    });

    // 切回英文同样即时生效。
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            if let Some(snapshot) = view.state.application_settings.as_mut() {
                snapshot.settings.language = "en".into();
            }
            view.sync_form_inputs(window, cx);
            assert_eq!(view.lang(), Lang::En);
        });
    });
}

#[gpui_kit::test]
fn connection_table_shares_snapshot_and_updates_before_render(cx: &mut TestAppContext) {
    use crate::domain::{Connection, ConnectionSnapshot, RealtimeEvent};
    let (_, holder, cx) = setup(cx);
    let view = holder.borrow().clone().unwrap();
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.state
                .apply_realtime(RealtimeEvent::Connections(ConnectionSnapshot {
                    upload_total: 0,
                    download_total: 0,
                    connection_count: 1,
                    connections: vec![Connection {
                        id: "test-connection".into(),
                        ..Default::default()
                    }],
                }));
            view.sync_connections(cx);
            let snapshot = view.state.connections.as_ref().unwrap();
            let table = view.connections_table.read(cx);
            assert!(std::sync::Arc::ptr_eq(
                snapshot,
                table.delegate().snapshot.as_ref().unwrap()
            ));
            assert_eq!(
                table.delegate().snapshot.as_ref().unwrap().connections[0].id,
                "test-connection"
            );
        });
    });
}

#[gpui_kit::test]
fn all_pages_render_at_minimum_window_size(cx: &mut TestAppContext) {
    let (_, holder, cx) = setup(cx);
    let view = holder.borrow().clone().unwrap();
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(960.), gpui_kit::px(640.)));
    for page in [
        crate::ui::Page::Home,
        crate::ui::Page::Proxies,
        crate::ui::Page::Rules,
        crate::ui::Page::Connections,
        crate::ui::Page::Profiles,
        crate::ui::Page::Logs,
        crate::ui::Page::Settings,
        crate::ui::Page::Ai,
    ] {
        cx.update(|_, cx| view.update(cx, |view, cx| view.navigate(page, cx)));
        cx.run_until_parked();
        let header = cx
            .debug_bounds("page-header")
            .expect("every page has one shared header");
        assert_eq!(header.size.height, gpui_kit::px(32.), "{page:?}");
        assert!(header.right() <= gpui_kit::px(960.), "{page:?}");
        let pane = cx.debug_bounds("content-pane").unwrap();
        assert_eq!(
            pane.bottom(),
            gpui_kit::px(632.),
            "only an 8px inset, no footer on {page:?}"
        );
        assert_eq!(
            pane.right(),
            gpui_kit::px(952.),
            "shared rounded content pane on {page:?}"
        );
    }
}

#[gpui_kit::test]
fn connection_close_button_dispatches_immediately(cx: &mut TestAppContext) {
    use crate::domain::{Connection, ConnectionSnapshot, RealtimeEvent, RuntimeCommand};
    cx.update(gpui_kit::init);
    let (tx, rx) = mpsc::sync_channel(32);
    let holder: ViewHolder = Default::default();
    let copy = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        *copy.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(960.), gpui_kit::px(640.)));
    let view = holder.borrow().clone().unwrap();
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.state
                .apply_realtime(RealtimeEvent::Connections(ConnectionSnapshot {
                    upload_total: 0,
                    download_total: 0,
                    connection_count: 1,
                    connections: vec![Connection {
                        id: "close-now".into(),
                        ..Default::default()
                    }],
                }));
            view.sync_connections(cx);
            view.navigate(crate::ui::Page::Connections, cx);
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        let table = view.read(cx).connections_table.clone();
        table.update(cx, |table, cx| table.scroll_to_col(8, cx));
    });
    cx.run_until_parked();
    let bounds = cx
        .debug_bounds("close-connection-0")
        .expect("close button must be visible");
    assert!(
        bounds.right() <= gpui_kit::px(936.),
        "last-column action must be reachable at minimum width"
    );
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
    assert!(matches!(rx.try_recv().unwrap().request,
        crate::ui::UiRequest::Runtime(RuntimeCommand::CloseConnection { id }) if id == "close-now"));
}

#[gpui_kit::test]
fn first_profile_activation_keeps_follow_up_reads_and_subscription(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (tx, rx) = mpsc::sync_channel(32);
    let (view, cx) = cx.add_window_view(|window, cx| MainView::new(tx, window, cx));
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.dispatch(
                crate::ui::UiAction::SelectProfile(
                    crate::domain::ProfileId::parse("first").unwrap(),
                ),
                cx,
            );
        })
    });
    let requests: Vec<_> = rx.try_iter().collect();
    assert!(requests.iter().any(|r| matches!(
        r.request,
        crate::ui::UiRequest::Runtime(crate::domain::RuntimeCommand::StartRealtime { .. })
    )));
    assert!(requests.iter().any(|r| matches!(
        r.request,
        crate::ui::UiRequest::Runtime(crate::domain::RuntimeCommand::GetMode)
    )));
    assert!(requests.iter().all(|r| r.operation_len == requests.len()));
}

#[gpui_kit::test]
fn populated_proxy_and_rule_lists_scroll_inside_viewport(cx: &mut TestAppContext) {
    use crate::{
        domain::{ProviderKind, ProviderSummary, ProxyGroup, RuleEntry},
        ui::Page,
    };
    let (_, holder, cx) = setup(cx);
    let view = holder.borrow().clone().unwrap();
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(960.), gpui_kit::px(640.)));
    cx.update(|_, cx| {
        view.update(cx, |view, _| {
            view.state.proxies = std::sync::Arc::new(crate::domain::ProxySnapshot {
                groups: vec![ProxyGroup {
                    name: "GLOBAL".into(),
                    kind: "Selector".into(),
                    selected: None,
                    members: (0..500).map(|i| format!("Node {i}")).collect(),
                }],
                ..Default::default()
            });
            view.state.mode = Some(crate::domain::RunMode::Global);
            view.state.rules = (0..500)
                .map(|i| RuleEntry {
                    kind: "DOMAIN".into(),
                    payload: format!("{i}.example.test"),
                    proxy: "DIRECT".into(),
                    size: 0,
                })
                .collect();
            view.state.providers = (0..20)
                .map(|i| ProviderSummary {
                    name: format!("Provider {i}"),
                    kind: ProviderKind::Rule,
                    vehicle: "HTTP".into(),
                    updated_at: String::new(),
                    item_count: 500,
                })
                .collect();
        })
    });
    for (page, id) in [(Page::Proxies, "proxy-list"), (Page::Rules, "rule-list")] {
        cx.update(|_, cx| view.update(cx, |view, cx| view.navigate(page, cx)));
        cx.run_until_parked();
        let bounds = cx.debug_bounds(id).expect("list must have bounds");
        if page == Page::Rules {
            let header = cx.debug_bounds("page-header").unwrap();
            let refresh = cx.debug_bounds("refresh-rules").unwrap();
            assert!(refresh.right() <= header.right());
            assert!(refresh.bottom() <= header.bottom());
        }
        assert!(
            bounds.size.height > gpui_kit::px(100.) && bounds.size.height < gpui_kit::px(550.),
            "{id}: {bounds:?}"
        );
        cx.simulate_event(gpui_kit::ScrollWheelEvent {
            position: bounds.center(),
            delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
                gpui_kit::px(0.),
                gpui_kit::px(-350.),
            )),
            modifiers: Default::default(),
            touch_phase: gpui_kit::TouchPhase::Moved,
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let view = view.read(cx);
            let scroll = if page == Page::Proxies {
                &view.proxy_page.read(cx).scroll
            } else {
                &view.rule_scroll
            };
            assert!(
                scroll.offset().y < gpui_kit::px(-100.),
                "{id} must move on wheel input: {:?}",
                scroll.offset()
            );
        });
    }
}

#[gpui_kit::test]
fn proxy_mode_buttons_dispatch_all_three_modes(cx: &mut TestAppContext) {
    use crate::{
        domain::{RunMode, RuntimeCommand},
        ui::{Page, UiRequest},
    };
    cx.update(gpui_kit::init);
    let (tx, rx) = mpsc::sync_channel(32);
    let holder: ViewHolder = Default::default();
    let copy = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        *copy.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    for mode in [RunMode::Global, RunMode::Direct, RunMode::Rule] {
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                // Fake a settled daemon snapshot for each independent click.
                view.state = crate::ui::UiState::default();
                view.state.page = Page::Proxies;
                view.state.mode = Some(if mode == RunMode::Rule {
                    RunMode::Global
                } else {
                    RunMode::Rule
                });
                cx.notify();
            })
        });
        cx.run_until_parked();
        let bounds = cx
            .debug_bounds(match mode {
                RunMode::Rule => "mode-Rule",
                RunMode::Global => "mode-Global",
                RunMode::Direct => "mode-Direct",
            })
            .expect("mode control visible even with no groups");
        cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
        assert!(rx.try_iter().any(|request| matches!(request.request, UiRequest::Runtime(RuntimeCommand::SetMode { mode: value }) if value == mode)));
    }
}

#[gpui_kit::test]
fn sidebar_navigation_and_collapsed_alignment(cx: &mut TestAppContext) {
    let (_, holder, cx) = setup(cx);
    let view = holder.borrow().clone().unwrap();
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(960.), gpui_kit::px(640.)));
    cx.run_until_parked();
    let profiles = cx.debug_bounds("nav-Profiles").unwrap();
    cx.simulate_click(profiles.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(view.read(cx).state.page, crate::ui::Page::Profiles));
    let toggle = cx.debug_bounds("sidebar-toggle").unwrap();
    cx.simulate_click(toggle.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    for _ in 0..90 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.update(|window, cx| window.simulate_next_frame(cx));
        cx.run_until_parked();
    }
    let toggle = cx.debug_bounds("sidebar-toggle").unwrap();
    let profiles = cx.debug_bounds("nav-Profiles").unwrap();
    let home = cx.debug_bounds("nav-Home").unwrap();
    assert!(
        (toggle.center().x - profiles.center().x).abs() < gpui_kit::px(1.),
        "toggle {toggle:?}, menu {profiles:?}"
    );
    assert_eq!(home.origin.x, profiles.origin.x);
    assert_eq!(home.size, profiles.size);
    cx.simulate_click(home.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(view.read(cx).state.page, crate::ui::Page::Home));
}

#[gpui_kit::test]
fn sidebar_animation_reverses_without_jumping(cx: &mut TestAppContext) {
    use gpui_kit::{Modifiers, px};
    use std::time::Duration;
    let (_, holder, cx) = setup(cx);
    let view = holder.borrow().clone().unwrap();
    cx.run_until_parked();
    let width = cx.debug_bounds("verge-sidebar").unwrap().size.width;
    assert_eq!(width, px(204.));
    let toggle = cx.debug_bounds("sidebar-toggle").unwrap();
    cx.simulate_click(toggle.center(), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(cx.debug_bounds("verge-sidebar").unwrap().size.width, width);
    cx.executor().advance_clock(Duration::from_millis(80));
    cx.update(|window, cx| window.simulate_next_frame(cx));
    cx.run_until_parked();
    let intermediate = cx.debug_bounds("verge-sidebar").unwrap().size.width;
    assert!(intermediate > px(56.) && intermediate < width);
    // Invoke the same handler while the hit target is moving. Synthetic pointer
    // down/up events can straddle different animation frames in the test runner.
    cx.update(|_, cx| view.update(cx, |view, cx| view.toggle_sidebar(cx)));
    cx.run_until_parked();
    let reversed = cx.debug_bounds("verge-sidebar").unwrap().size.width;
    assert!(
        reversed > px(56.) && reversed < width,
        "reverse must preserve an intermediate width"
    );
    for _ in 0..90 {
        cx.executor().advance_clock(Duration::from_millis(16));
        cx.update(|window, cx| window.simulate_next_frame(cx));
        cx.run_until_parked();
    }
    assert_eq!(cx.debug_bounds("verge-sidebar").unwrap().size.width, width);

    // Native GPUI motion preference resolves to the final layout immediately.
    cx.update(|_, cx| cx.set_reduce_motion(true));
    let toggle = cx.debug_bounds("sidebar-toggle").unwrap();
    cx.simulate_click(toggle.center(), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        cx.debug_bounds("verge-sidebar").unwrap().size.width,
        px(56.)
    );
}

#[gpui_kit::test]
fn network_dialogs_render_and_save_only_after_backend_success(cx: &mut TestAppContext) {
    use crate::{
        domain::{AppCommand, AppCommandOutput, AppCommandResult, CoreNetworkSettings},
        ui::UiRequest,
    };
    cx.update(gpui_kit::init);
    let (tx, rx) = mpsc::sync_channel(32);
    let holder: ViewHolder = Default::default();
    let copy = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        *copy.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(1080.), gpui_kit::px(800.)));
    cx.run_until_parked();
    for dns in [false, true] {
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.state.core_network_settings = Some(CoreNetworkSettings::default());
                view.open_core_network_dialog(dns, window, cx);
            })
        });
        cx.run_until_parked();
        assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
        for _ in 0..25 {
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(16));
            cx.update(|window, cx| window.simulate_next_frame(cx));
            cx.run_until_parked();
        }
        if dns {
            let toggle = cx.debug_bounds("dialog-dns-enabled").unwrap();
            cx.simulate_click(toggle.center(), gpui_kit::Modifiers::default());
            cx.run_until_parked();
        }
        let bounds = cx
            .debug_bounds("network-dialog-save")
            .expect("network save button");
        cx.simulate_mouse_move(bounds.center(), None, gpui_kit::Modifiers::default());
        cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
        cx.run_until_parked();
        let envelope = rx.try_recv().expect("save sends a request");
        let UiRequest::Profile(command @ AppCommand::UpdateCoreNetworkSettings { .. }) =
            envelope.request.clone()
        else {
            panic!("wrong save command");
        };
        if let AppCommand::UpdateCoreNetworkSettings { settings } = &command {
            assert_eq!(
                settings.dns_override, dns,
                "dialog toggles update retained draft"
            );
        }
        assert!(
            cx.update(|window, cx| window.has_active_dialog(cx)),
            "keep draft until backend confirms"
        );
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                let response = crate::ui::UiResponseEnvelope::for_request(
                    &envelope,
                    crate::ui::UiResponse::Profile {
                        request: command.clone(),
                        result: Ok(AppCommandResult {
                            output: AppCommandOutput::CoreNetworkSettings(
                                CoreNetworkSettings::default(),
                            ),
                            summary: "saved".into(),
                        }),
                    },
                );
                view.settings_dialog_response(&response, window, cx);
                view.state.apply_response_envelope(response);
                view.sync_form_inputs(window, cx);
            })
        });
        cx.run_until_parked();
        assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));
        while rx.try_recv().is_ok() {}
    }
}
#[test]
fn delay_timeouts_stay_on_the_node_but_core_failures_still_notify() {
    use crate::domain::{AppError, ErrorCode, RuntimeCommand};
    use crate::ui::UiResponse;
    for code in [
        ErrorCode::RequestTimeout,
        ErrorCode::ProxyDelayFailed,
        ErrorCode::CoreUnavailable,
    ] {
        let response = UiResponse::Runtime {
            request: RuntimeCommand::TestProxyDelay {
                proxy: "Node".into(),
                url: "https://example.invalid".into(),
                timeout_ms: 5000,
            },
            result: Err(AppError::new(code, "test failed")),
        };
        assert_eq!(
            super::toast_for(crate::i18n::Lang::En, &response).is_some(),
            code == ErrorCode::CoreUnavailable
        );
    }
}

#[test]
fn unknown_error_has_no_misleading_recovery_hint() {
    let error: crate::domain::AppError =
        serde_json::from_str(r#"{"code":"future_busy","message":"Try again later"}"#).unwrap();
    assert_eq!(super::recovery_hint(Lang::ZhCn, &error), None);
    assert_eq!(error.message, "Try again later");
}

#[gpui_kit::test]
fn cmd_w_closes_main_window_with_focused_input(cx: &mut TestAppContext) {
    let (_, view_holder, visual) = setup(cx);
    let view = view_holder.borrow().clone().unwrap();
    visual.update(|window, cx| {
        super::bind_window_actions(cx);
        view.update(cx, |view, cx| view.open_import_dialog(window, cx));
    });
    visual.run_until_parked();
    visual.simulate_keystrokes("cmd-w");
    assert!(
        cx.windows().is_empty(),
        "Cmd+W closes the window even with a dialog/input focused"
    );
}

#[gpui_kit::test]
fn cmd_w_closes_window_from_main_view(cx: &mut TestAppContext) {
    let (_, _, visual) = setup(cx);
    visual.update(|_, cx| super::bind_window_actions(cx));
    visual.run_until_parked();
    visual.simulate_keystrokes("cmd-w");
    assert!(cx.windows().is_empty());
}

#[gpui_kit::test]
fn system_proxy_dialog_keeps_draft_until_success_and_fits_small_window(cx: &mut TestAppContext) {
    use crate::{domain::*, ui::UiRequest};
    cx.update(gpui_kit::init);
    let (tx, rx) = mpsc::sync_channel(32);
    let holder: ViewHolder = Default::default();
    let copy = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        *copy.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(960.), gpui_kit::px(640.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.state.application_settings = Some(ApplicationSettingsSnapshot {
                settings: ApplicationSettings::default(),
                data_directory: "/tmp/test".into(),
                app_version: None,
            });
            view.open_system_proxy_dialog(window, cx);
        })
    });
    cx.run_until_parked();
    for _ in 0..25 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.update(|window, cx| window.simulate_next_frame(cx));
        cx.run_until_parked();
    }
    let bounds = cx.debug_bounds("system-proxy-dialog-scroll").unwrap();
    let before = cx.debug_bounds("proxy-bypass-editor").unwrap();
    cx.simulate_event(gpui_kit::ScrollWheelEvent {
        position: bounds.center(),
        delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(
            gpui_kit::px(0.),
            gpui_kit::px(-500.),
        )),
        modifiers: Default::default(),
        touch_phase: gpui_kit::TouchPhase::Moved,
    });
    cx.run_until_parked();
    let after = cx.debug_bounds("proxy-bypass-editor").unwrap();
    assert!(
        after.top() < before.top(),
        "bounds={bounds:?}, before={before:?}, after={after:?}"
    );
    assert!(
        after.bottom() <= bounds.bottom(),
        "custom bypass editor must be reachable"
    );
    cx.simulate_event(gpui_kit::ScrollWheelEvent {
        position: bounds.center(),
        delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(500.))),
        modifiers: Default::default(),
        touch_phase: gpui_kit::TouchPhase::Moved,
    });
    cx.run_until_parked();
    let toggle = cx.debug_bounds("proxy-pac-mode").unwrap();
    cx.simulate_click(toggle.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    let save = cx.debug_bounds("proxy-dialog-save").unwrap();
    assert!(save.bottom() <= gpui_kit::px(640.));
    cx.simulate_click(save.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    let envelope = rx.try_recv().unwrap();
    let UiRequest::Profile(AppCommand::UpdateSystemProxySettings { settings }) =
        envelope.request.clone()
    else {
        panic!("wrong request")
    };
    assert!(settings.pac_mode);
    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            assert!(
                !view
                    .state
                    .application_settings
                    .as_ref()
                    .unwrap()
                    .settings
                    .system_proxy
                    .pac_mode
            );
            view.state
                .application_settings
                .as_mut()
                .unwrap()
                .settings
                .system_proxy = settings;
            let response = crate::ui::UiResponseEnvelope::for_request(
                &envelope,
                crate::ui::UiResponse::Profile {
                    request: match envelope.request.clone() {
                        UiRequest::Profile(command) => command,
                        _ => unreachable!(),
                    },
                    result: Ok(AppCommandResult {
                        output: AppCommandOutput::None,
                        summary: "saved".into(),
                    }),
                },
            );
            view.settings_dialog_response(&response, window, cx);
            view.sync_form_inputs(window, cx);
        })
    });
    cx.run_until_parked();
    assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));
}

#[gpui_kit::test]
fn daemon_recovery_preserves_drafts_and_never_replays_writes(cx: &mut TestAppContext) {
    use crate::{
        domain::RunMode,
        ui::{Page, UiAction},
    };
    cx.update(gpui_kit::init);
    let (tx, rx) = mpsc::sync_channel(32);
    let holder = Rc::new(RefCell::new(None));
    let slot = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        *slot.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.yaml_editor.update(cx, |input, cx| {
                input.set_value("draft: retained", window, cx)
            });
            view.dispatch(UiAction::SetMode(RunMode::Global), cx);
            let original = rx.try_recv().unwrap();
            assert!(!view.state.pending.is_empty());
            view.daemon_disconnected(window, cx);
            assert!(view.state.pending.is_empty());
            view.dispatch(UiAction::SetMode(RunMode::Direct), cx);
            view.navigate(Page::Profiles, cx);
            assert_eq!(view.state.page, Page::Profiles);
            assert!(rx.try_recv().is_err());
            assert_eq!(
                view.yaml_editor.read(cx).value().as_str(),
                "draft: retained"
            );
            let (new_tx, new_rx) = mpsc::sync_channel(32);
            view.requests = new_tx;
            view.state.connection_notice = None;
            view.dispatch(UiAction::RefreshProfiles, cx);
            let recovered = new_rx.try_recv().unwrap();
            assert!(recovered.request_id > original.request_id);
            assert_eq!(
                recovered.request.risk(),
                crate::domain::CommandRisk::ReadOnly
            );
            assert!(new_rx.try_recv().is_err());
        })
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn welcome_and_repeated_navigation_share_pending_home_queries(cx: &mut TestAppContext) {
    use crate::{
        domain::{ProfileId, RuntimeCommand},
        ui::{Page, UiAction, UiRequest},
    };
    cx.update(gpui_kit::init);
    let (tx, rx) = mpsc::sync_channel(32);
    let (view, cx) = cx.add_window_view(|window, cx| MainView::new(tx, window, cx));
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.state.selected_profile = Some(ProfileId::parse("test").unwrap());
            view.refresh_after_connect(cx);
            let initial: Vec<_> = rx.try_iter().collect();
            let queries = initial
                .iter()
                .filter(|e| {
                    matches!(
                        &e.request,
                        UiRequest::Runtime(
                            RuntimeCommand::GetMode
                                | RuntimeCommand::GetNetworkSettings
                                | RuntimeCommand::ListProxyGroups
                                | RuntimeCommand::ListRules
                                | RuntimeCommand::ListProviders
                        )
                    )
                })
                .count();
            assert_eq!(
                queries, 5,
                "welcome should request each Mihomo snapshot once"
            );
            for (i, request) in initial.iter().enumerate() {
                assert!(
                    !initial[..i]
                        .iter()
                        .any(|other| other.request == request.request)
                );
            }
            for _ in 0..10 {
                for page in [
                    Page::Home,
                    Page::Proxies,
                    Page::Rules,
                    Page::Settings,
                    Page::Profiles,
                    Page::Logs,
                ] {
                    view.navigate(page, cx);
                    view.refresh_current_page(cx);
                }
            }
            assert!(rx.try_recv().is_err(), "pending snapshots should be reused");
            assert!(view.state.last_error.is_none());
            let mode = initial
                .iter()
                .find(|e| e.request == UiRequest::Runtime(RuntimeCommand::GetMode))
                .unwrap();
            view.state
                .apply_response_envelope(crate::ui::UiResponseEnvelope::for_request(
                    mode,
                    crate::ui::UiResponse::Runtime {
                        request: RuntimeCommand::GetMode,
                        result: Err(crate::domain::AppError::new(
                            crate::domain::ErrorCode::RequestTimeout,
                            "test timeout",
                        )),
                    },
                ));
            view.dispatch(UiAction::RefreshProxies, cx);
            let retry = rx.try_recv().unwrap();
            assert_eq!(retry.request, mode.request);
            assert!(retry.request_id > mode.request_id);
            assert!(rx.try_recv().is_err());

            // A mutation invalidates sharing with earlier reads; its follow-up
            // refresh must observe the updated state, not reuse the old request.
            view.dispatch(UiAction::SetMode(crate::domain::RunMode::Global), cx);
            assert!(matches!(
                rx.try_recv().unwrap().request,
                UiRequest::Runtime(RuntimeCommand::SetMode { .. })
            ));
            view.dispatch(UiAction::RefreshProxies, cx);
            let after_write: Vec<_> = rx.try_iter().collect();
            assert_eq!(after_write.len(), 2);
            assert!(after_write.iter().all(|e| e.request_id > retry.request_id));

            // A new daemon session must receive fresh reads and subscriptions.
            view.state.disconnect("reconnecting".into());
            view.dispatch(UiAction::SetMode(crate::domain::RunMode::Rule), cx);
            assert!(view.state.last_error.is_some());
            assert!(rx.try_recv().is_err(), "offline actions must stay local");
            // Welcome clears the notice, then runs this refresh chain. Starting
            // the accepted reads already clears the offline-action Alert.
            view.state.connection_notice.take();
            view.refresh_after_connect(cx);
            assert!(view.state.last_error.is_none());
            assert_eq!(rx.try_iter().count(), initial.len());
        })
    });
}

#[gpui_kit::test]
fn daemon_requests_are_bounded_while_responses_are_stalled(cx: &mut TestAppContext) {
    use crate::ui::UiAction;
    cx.update(gpui_kit::init);
    let (tx, rx) = mpsc::sync_channel(32);
    let (view, cx) = cx.add_window_view(|window, cx| MainView::new(tx, window, cx));
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            for _ in 0..100 {
                view.dispatch(UiAction::SetMode(crate::domain::RunMode::Rule), cx);
            }
            assert_eq!(rx.try_iter().count(), 32);
            assert_eq!(view.state.pending_request_count(), 32);
        })
    });
}

#[gpui_kit::test]
fn connection_details_and_close_all_use_real_overlays(cx: &mut TestAppContext) {
    use crate::domain::{Connection, RuntimeCommand};
    use crate::ui::{UiAction, UiRequest};
    cx.update(gpui_kit::init);
    let (tx, rx) = mpsc::sync_channel(32);
    let holder = Rc::new(RefCell::new(None));
    let slot = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        *slot.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.open_connection_details(
                Connection {
                    host: "test.example".into(),
                    ..Default::default()
                },
                window,
                cx,
            );
            assert!(window.has_active_sheet(cx));
            view.confirm_close_all_connections(window, cx);
            assert!(window.has_active_dialog(cx));
            assert!(rx.try_recv().is_err());
            view.dispatch(UiAction::CloseAllConnections, cx);
            assert!(
                rx.try_recv().is_err(),
                "unsupported daemon must not receive new command"
            );
            view.state
                .daemon_capabilities
                .push(crate::ipc::protocol::CLOSE_ALL_CONNECTIONS.into());
            view.dispatch_confirmed(UiAction::CloseAllConnections, cx);
            assert!(matches!(
                rx.try_recv().unwrap().request,
                UiRequest::Runtime(RuntimeCommand::CloseAllConnections)
            ));
        })
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn merge_preview_keeps_editor_draft_and_opens_a_result_dialog(cx: &mut TestAppContext) {
    use crate::domain::ProfileId;
    let (_, holder, cx) = setup(cx);
    let view = holder.borrow().clone().unwrap();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.merge_editor
                .update(cx, |editor, cx| editor.set_value("rules: []", window, cx));
            let id = ProfileId::parse("draft").unwrap();
            view.sheet_state.update(cx, |state, _| {
                state.request_id = Some(42);
                state.pending_merged = Some(id.clone());
                state.merge_preview_dialog = true;
            });
            view.profile_sheet_response(
                &crate::ui::UiResponseEnvelope {
                    request_id: Some(42),
                    operation_id: Some(42),
                    response: crate::ui::UiResponse::Profile {
                        request: crate::domain::AppCommand::PreviewMergeConfig {
                            id: id.clone(),
                            yaml: "rules: []".into(),
                        },
                        result: Ok(crate::domain::AppCommandResult {
                            output: crate::domain::AppCommandOutput::MergedProfileYaml {
                                id,
                                yaml: "mode: global".into(),
                            },
                            summary: String::new(),
                        }),
                    },
                },
                window,
                cx,
            );
            assert!(window.has_active_dialog(cx));
            assert_eq!(view.merge_editor.read(cx).value().as_str(), "rules: []");
            assert_eq!(view.merged_editor.read(cx).value().as_str(), "mode: global");
        })
    });
    cx.run_until_parked();
}

#[gpui_kit::test]
fn page_headers_stay_visible_and_scroll_offsets_are_isolated(cx: &mut TestAppContext) {
    use crate::domain::{
        ApplicationSettings, ApplicationSettingsSnapshot, Profile, ProfileId, ProfileSource,
        UpdatePolicy,
    };
    use gpui_kit::{ScrollDelta, ScrollWheelEvent, TouchPhase, point, px, size};
    let (_, holder, cx) = setup(cx);
    let view = holder.borrow().clone().unwrap();
    cx.simulate_resize(size(px(960.), px(640.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.state.application_settings = Some(ApplicationSettingsSnapshot {
                settings: ApplicationSettings::default(),
                data_directory: "/tmp/verge-test".into(),
                app_version: None,
            });
            view.state.profiles = (0..12)
                .map(|i| {
                    Profile::new(
                        ProfileId::parse(format!("p-{i}")).unwrap(),
                        format!("Profile {i}"),
                        ProfileSource::Local,
                        UpdatePolicy::Manual,
                        0,
                        None,
                    )
                    .unwrap()
                })
                .collect();
            view.state.page = crate::ui::Page::Profiles;
            view.sync_form_inputs(window, cx);
            cx.notify();
        })
    });
    cx.run_until_parked();
    let header = cx.debug_bounds("page-header").unwrap();
    for page in [crate::ui::Page::Profiles, crate::ui::Page::Settings] {
        cx.update(|_, cx| {
            view.update(cx, |view, cx| {
                view.state.page = page;
                cx.notify();
            })
        });
        cx.run_until_parked();
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(800.), px(480.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-600.))),
            modifiers: Default::default(),
            touch_phase: TouchPhase::Moved,
        });
        cx.run_until_parked();
        assert_eq!(cx.debug_bounds("page-header").unwrap(), header);
    }
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.settings_category = crate::pages::settings::SettingsCategory::Updates;
            cx.notify();
        })
    });
    cx.run_until_parked();
    let core = cx
        .debug_bounds("settings-category-settings.group.app_update")
        .unwrap();
    assert!(core.top() >= header.bottom());
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.state.page = crate::ui::Page::Home;
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("home-profile-name").unwrap().top() >= header.bottom());
}

#[gpui_kit::test]
fn log_limit_draft_does_not_write_until_committed(cx: &mut TestAppContext) {
    use crate::{
        domain::{ApplicationSettings, ApplicationSettingsSnapshot},
        ui::UiRequest,
    };
    use gpui_kit::component::input::InputEvent;
    cx.update(gpui_kit::init);
    let (tx, rx) = mpsc::sync_channel(32);
    let holder: ViewHolder = Default::default();
    let slot = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        *slot.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.state.application_settings = Some(ApplicationSettingsSnapshot {
                settings: ApplicationSettings::default(),
                data_directory: String::new(),
                app_version: None,
            });
            view.log_limit.update(cx, |input, cx| {
                input.set_value("1", window, cx);
                cx.emit(InputEvent::Change);
            });
        })
    });
    cx.run_until_parked();
    assert!(rx.try_recv().is_err());
    cx.update(|_, cx| {
        view.read(cx)
            .log_limit
            .clone()
            .update(cx, |_, cx| cx.emit(InputEvent::Blur))
    });
    cx.run_until_parked();
    assert!(
        rx.try_recv().is_err(),
        "invalid limit must not reach the daemon"
    );
    cx.update(|window, cx| {
        view.read(cx).log_limit.clone().update(cx, |input, cx| {
            input.set_value("1500", window, cx);
            cx.emit(InputEvent::Blur);
        })
    });
    cx.run_until_parked();
    assert!(
        matches!(rx.try_recv().unwrap().request, UiRequest::Profile(crate::domain::AppCommand::UpdateApplicationSettings { settings }) if settings.log_limit == 1500)
    );
}

#[gpui_kit::test]
fn late_settings_response_cannot_close_another_dialog(cx: &mut TestAppContext) {
    use crate::{
        domain::{AppCommand, AppCommandOutput, AppCommandResult, CoreNetworkSettings},
        ui::{UiAction, UiResponse, UiResponseEnvelope},
    };
    cx.update(gpui_kit::init);
    let (tx, rx) = mpsc::sync_channel(32);
    let holder: ViewHolder = Default::default();
    let slot = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        *slot.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            let focus = window.focused(cx).unwrap();
            let id = view
                .dispatch_sheet_request(
                    UiAction::UpdateCoreNetworkSettings(CoreNetworkSettings::default()),
                    cx,
                )
                .unwrap();
            view.network_form.pending_dialog = Some((id, focus));
        })
    });
    let request = rx.try_recv().unwrap();
    cx.update(|window, cx| {
        window.open_dialog(cx, |dialog, _, _| dialog.title("Unrelated confirmation"))
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            let response = UiResponseEnvelope::for_request(
                &request,
                UiResponse::Profile {
                    request: AppCommand::UpdateCoreNetworkSettings {
                        settings: CoreNetworkSettings::default(),
                    },
                    result: Ok(AppCommandResult {
                        output: AppCommandOutput::None,
                        summary: String::new(),
                    }),
                },
            );
            view.settings_dialog_response(&response, window, cx);
        })
    });
    assert!(cx.update(|window, cx| window.has_active_dialog(cx)));
}

#[gpui_kit::test]
fn navigation_shortcuts_follow_sidebar_including_ai(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.bind_keys(crate::view::navigation::key_bindings("cmd"));
    });
    let (tx, _rx) = mpsc::sync_channel(64);
    let holder: ViewHolder = Default::default();
    let slot = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        *slot.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    for (index, (page, _, _)) in crate::view::navigation::ENTRIES.iter().enumerate() {
        cx.simulate_keystrokes(&format!("cmd-{}", index + 1));
        cx.run_until_parked();
        assert_eq!(cx.update(|_, cx| view.read(cx).state.page), *page);
    }
}

#[gpui_kit::test]
fn cancelling_network_save_clears_its_request_marker(cx: &mut TestAppContext) {
    use crate::domain::CoreNetworkSettings;
    cx.update(gpui_kit::init);
    let (tx, _rx) = mpsc::sync_channel(32);
    let holder: ViewHolder = Default::default();
    let slot = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        *slot.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.state.core_network_settings = Some(CoreNetworkSettings::default());
            view.open_core_network_dialog(true, window, cx);
        })
    });
    cx.run_until_parked();
    for _ in 0..25 {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.update(|window, cx| window.simulate_next_frame(cx));
        cx.run_until_parked();
    }
    let save = cx.debug_bounds("network-dialog-save").unwrap();
    cx.simulate_click(save.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.update(|_, cx| view.read(cx).network_form.pending_dialog.is_some()));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(cx.update(|_, cx| view.read(cx).network_form.pending_dialog.is_none()));
}
