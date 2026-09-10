//! Root / Dialog / Sheet 机制的 UI 测试。
//!
//! 用 gpui 的 `add_window_view`（与 gpui-component 官方测试一致的环境）验证：
//! 1. dialog/sheet 打开后 `Root::render_*_layer` 必须存在（MainView 已挂载
//!    overlay 层——此前未挂载导致"点了没反应"）；
//! 2. sheet builder 渲染时不能读 MainView 实体（已用 SheetState 解耦）。

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use gpui::{AppContext as _, Styled as _, TestAppContext, VisualTestContext};
use gpui_component::{ActiveTheme as _, Root, WindowExt as _};

use crate::i18n::Lang;
use crate::view::MainView;

type ViewHolder = Rc<RefCell<Option<gpui::Entity<MainView>>>>;

/// 创建窗口：MainView + Root（与真实 main() 同构）。
fn setup(cx: &mut TestAppContext) -> (gpui::Entity<Root>, ViewHolder, &mut VisualTestContext) {
    cx.update(gpui_component::init);
    let (request_tx, _request_rx) = mpsc::channel();
    let view_holder: ViewHolder = Default::default();
    let holder = view_holder.clone();
    let (root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(request_tx, window, cx));
        *holder.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx).bg(cx.theme().background)
    });
    (root, view_holder, cx)
}

#[gpui::test]
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

    let (has_dialog, layer) = cx.update(|window, cx| {
        (
            window.has_active_dialog(cx),
            Root::render_dialog_layer(window, cx).is_some(),
        )
    });
    assert!(has_dialog, "open_import_dialog 后应有活跃 dialog");
    assert!(
        layer,
        "dialog 打开后渲染层必须存在（MainView 已挂载 overlay）"
    );
}

#[gpui::test]
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

    let (has_sheet, layer) = cx.update(|window, cx| {
        (
            window.has_active_sheet(cx),
            Root::render_sheet_layer(window, cx).is_some(),
        )
    });
    assert!(has_sheet, "open_yaml_sheet 后应有活跃 sheet");
    assert!(
        layer,
        "sheet 打开后渲染层必须存在（MainView 已挂载 overlay）"
    );
}

#[gpui::test]
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

    let (has_sheet, layer) = cx.update(|window, cx| {
        (
            window.has_active_sheet(cx),
            Root::render_sheet_layer(window, cx).is_some(),
        )
    });
    assert!(has_sheet && layer, "merge sheet 应打开且渲染层存在");
}

#[gpui::test]
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

    let (has_sheet, layer) = cx.update(|window, cx| {
        (
            window.has_active_sheet(cx),
            Root::render_sheet_layer(window, cx).is_some(),
        )
    });
    assert!(has_sheet && layer, "merged sheet 应打开且渲染层存在");
}

/// 语言切换：设置写入后 MainView::lang 立即反映新语言，sync_form_inputs
/// 同步重设输入框占位（状态驱动，无需重启；占位内容正确性由 i18n 查表测试覆盖）。
#[gpui::test]
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

#[gpui::test]
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

#[gpui::test]
fn all_pages_render_at_minimum_window_size(cx: &mut TestAppContext) {
    let (_, holder, cx) = setup(cx);
    let view = holder.borrow().clone().unwrap();
    cx.simulate_resize(gpui::size(gpui::px(960.), gpui::px(640.)));
    for page in [
        crate::ui::Page::Home,
        crate::ui::Page::Proxies,
        crate::ui::Page::Rules,
        crate::ui::Page::Connections,
        crate::ui::Page::Profiles,
        crate::ui::Page::Logs,
        crate::ui::Page::Settings,
    ] {
        cx.update(|_, cx| view.update(cx, |view, cx| view.navigate(page, cx)));
        cx.run_until_parked();
        let header = cx
            .debug_bounds("page-header")
            .expect("every page has one shared header");
        assert_eq!(header.size.height, gpui::px(32.), "{page:?}");
        assert!(header.right() <= gpui::px(960.), "{page:?}");
    }
}

#[gpui::test]
fn connection_close_button_dispatches_immediately(cx: &mut TestAppContext) {
    use crate::domain::{Connection, ConnectionSnapshot, RealtimeEvent, RuntimeCommand};
    cx.update(gpui_component::init);
    let (tx, rx) = mpsc::channel();
    let holder: ViewHolder = Default::default();
    let copy = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        *copy.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.simulate_resize(gpui::size(gpui::px(960.), gpui::px(640.)));
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
    let bounds = cx
        .debug_bounds("close-connection-0")
        .expect("close button must be visible");
    assert!(
        bounds.right() <= gpui::px(936.),
        "close action must fit at minimum width"
    );
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    assert!(matches!(rx.try_recv().unwrap().request,
        crate::ui::UiRequest::Runtime(RuntimeCommand::CloseConnection { id }) if id == "close-now"));
}

#[gpui::test]
fn first_profile_activation_keeps_follow_up_reads_and_subscription(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let (tx, rx) = mpsc::channel();
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

#[gpui::test]
fn populated_proxy_and_rule_lists_scroll_inside_viewport(cx: &mut TestAppContext) {
    use crate::{
        domain::{ProviderKind, ProviderSummary, ProxyGroup, RuleEntry},
        ui::Page,
    };
    let (_, holder, cx) = setup(cx);
    let view = holder.borrow().clone().unwrap();
    cx.simulate_resize(gpui::size(gpui::px(960.), gpui::px(640.)));
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
            bounds.size.height > gpui::px(100.) && bounds.size.height < gpui::px(550.),
            "{id}: {bounds:?}"
        );
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: bounds.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-350.))),
            modifiers: Default::default(),
            touch_phase: gpui::TouchPhase::Moved,
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
                scroll.offset().y < gpui::px(-100.),
                "{id} must move on wheel input: {:?}",
                scroll.offset()
            );
        });
    }
}

#[gpui::test]
fn proxy_mode_buttons_dispatch_all_three_modes(cx: &mut TestAppContext) {
    use crate::{
        domain::{RunMode, RuntimeCommand},
        ui::{Page, UiRequest},
    };
    cx.update(gpui_component::init);
    let (tx, rx) = mpsc::channel();
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
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        assert!(rx.try_iter().any(|request| matches!(request.request, UiRequest::Runtime(RuntimeCommand::SetMode { mode: value }) if value == mode)));
    }
}

#[gpui::test]
fn sidebar_navigation_and_collapsed_alignment(cx: &mut TestAppContext) {
    let (_, holder, cx) = setup(cx);
    let view = holder.borrow().clone().unwrap();
    cx.simulate_resize(gpui::size(gpui::px(960.), gpui::px(640.)));
    cx.run_until_parked();
    let profiles = cx.debug_bounds("nav-Profiles").unwrap();
    cx.simulate_click(profiles.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(view.read(cx).state.page, crate::ui::Page::Profiles));
    let toggle = cx.debug_bounds("sidebar-toggle").unwrap();
    cx.simulate_click(toggle.center(), gpui::Modifiers::default());
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
        (toggle.center().x - profiles.center().x).abs() < gpui::px(1.),
        "toggle {toggle:?}, menu {profiles:?}"
    );
    assert_eq!(home.origin.x, profiles.origin.x);
    assert_eq!(home.size, profiles.size);
    cx.simulate_click(home.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(view.read(cx).state.page, crate::ui::Page::Home));
}

#[gpui::test]
fn sidebar_animation_reverses_without_jumping(cx: &mut TestAppContext) {
    use gpui::{Modifiers, px};
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

#[gpui::test]
fn network_dialogs_render_and_save_only_after_backend_success(cx: &mut TestAppContext) {
    use crate::{
        domain::{AppCommand, AppCommandOutput, AppCommandResult, CoreNetworkSettings},
        ui::UiRequest,
    };
    cx.update(gpui_component::init);
    let (tx, rx) = mpsc::channel();
    let holder: ViewHolder = Default::default();
    let copy = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        *copy.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    cx.simulate_resize(gpui::size(gpui::px(1080.), gpui::px(800.)));
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
            cx.simulate_click(toggle.center(), gpui::Modifiers::default());
            cx.run_until_parked();
        }
        let bounds = cx
            .debug_bounds("network-dialog-save")
            .expect("network save button");
        cx.simulate_mouse_move(bounds.center(), None, gpui::Modifiers::default());
        cx.simulate_click(bounds.center(), gpui::Modifiers::default());
        cx.run_until_parked();
        let request = rx.try_recv().expect("save sends a request").request;
        let UiRequest::Profile(command @ AppCommand::UpdateCoreNetworkSettings { .. }) = request
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
                view.state.apply_profile(
                    &command,
                    AppCommandResult {
                        output: AppCommandOutput::CoreNetworkSettings(
                            CoreNetworkSettings::default(),
                        ),
                        summary: "saved".into(),
                    },
                );
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

#[gpui::test]
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

#[gpui::test]
fn cmd_w_closes_window_from_main_view(cx: &mut TestAppContext) {
    let (_, _, visual) = setup(cx);
    visual.update(|_, cx| super::bind_window_actions(cx));
    visual.run_until_parked();
    visual.simulate_keystrokes("cmd-w");
    assert!(cx.windows().is_empty());
}

#[gpui::test]
fn system_proxy_dialog_keeps_draft_until_success_and_fits_small_window(cx: &mut TestAppContext) {
    use crate::{domain::*, ui::UiRequest};
    cx.update(gpui_component::init);
    let (tx, rx) = mpsc::channel();
    let holder: ViewHolder = Default::default();
    let copy = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        *copy.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    cx.simulate_resize(gpui::size(gpui::px(960.), gpui::px(640.)));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.state.application_settings = Some(ApplicationSettingsSnapshot {
                settings: ApplicationSettings::default(),
                data_directory: "/tmp/test".into(),
                app_version: None,
            });
            view.state.daemon_capabilities =
                vec![crate::ipc::protocol::UNIFIED_SYSTEM_PROXY.into()];
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
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: bounds.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-500.))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
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
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: bounds.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(500.))),
        modifiers: Default::default(),
        touch_phase: gpui::TouchPhase::Moved,
    });
    cx.run_until_parked();
    let toggle = cx.debug_bounds("proxy-pac-mode").unwrap();
    cx.simulate_click(toggle.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    let save = cx.debug_bounds("proxy-dialog-save").unwrap();
    assert!(save.bottom() <= gpui::px(640.));
    cx.simulate_click(save.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    let UiRequest::Profile(AppCommand::UpdateSystemProxySettings { settings }) =
        rx.try_recv().unwrap().request
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
            view.sync_form_inputs(window, cx);
        })
    });
    cx.run_until_parked();
    assert!(!cx.update(|window, cx| window.has_active_dialog(cx)));
}

#[gpui::test]
fn unified_proxy_writes_do_not_reach_an_older_daemon(cx: &mut TestAppContext) {
    use crate::ui::UiAction;
    cx.update(gpui_component::init);
    let (tx, rx) = mpsc::channel();
    let holder = Rc::new(RefCell::new(None));
    let slot = holder.clone();
    let (_window, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        *slot.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            for action in [
                UiAction::SetSystemProxy { enabled: true },
                UiAction::UpdateSystemProxySettings(Box::default()),
            ] {
                view.dispatch(action, cx);
                assert!(rx.try_recv().is_err());
                assert_eq!(
                    view.state.last_error.as_ref().unwrap().code,
                    crate::domain::ErrorCode::Conflict
                );
            }
            view.state
                .daemon_capabilities
                .push(crate::ipc::protocol::UNIFIED_SYSTEM_PROXY.into());
            view.dispatch(UiAction::SetSystemProxy { enabled: true }, cx);
            assert!(matches!(
                rx.try_recv().unwrap().request,
                crate::ui::UiRequest::SystemProxy(crate::domain::SystemProxyCommand::SetEnabled {
                    enabled: true
                })
            ));
        })
    });
}
