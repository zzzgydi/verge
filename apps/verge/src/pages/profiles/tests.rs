use std::{cell::RefCell, rc::Rc, sync::mpsc};

use gpui_kit::component::{
    Root, WindowExt as _,
    dialog::{Cancel, Confirm},
};
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext};

use crate::{
    domain::{AppCommand, AppCommandOutput, AppCommandResult, AppError, ErrorCode, ProfileId},
    ui::{UiRequest, UiRequestEnvelope, UiResponse, UiResponseEnvelope},
    view::MainView,
};

fn setup(
    cx: &mut TestAppContext,
) -> (
    gpui_kit::Entity<MainView>,
    mpsc::Receiver<UiRequestEnvelope>,
    &mut VisualTestContext,
) {
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
    (view, rx, cx)
}

fn response(
    request: &UiRequestEnvelope,
    result: Result<AppCommandOutput, AppError>,
) -> UiResponseEnvelope {
    let UiRequest::Profile(command) = &request.request else {
        panic!("expected Profile request")
    };
    UiResponseEnvelope::for_request(
        request,
        UiResponse::Profile {
            request: command.clone(),
            result: result.map(|output| AppCommandResult {
                output,
                summary: String::new(),
            }),
        },
    )
}

fn deliver(
    view: &mut MainView,
    response: UiResponseEnvelope,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::Context<MainView>,
) {
    view.profile_sheet_response(&response, window, cx);
    view.state.apply_response_envelope(response);
    cx.notify();
}

#[gpui_kit::test]
fn reopened_editors_wait_for_their_own_response(cx: &mut TestAppContext) {
    let (view, rx, cx) = setup(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.state.merge_yaml = Some("rules: []".into());
            view.open_merge_sheet(window, cx);
            let old = rx.try_recv().unwrap();
            window.close_sheet(cx);
            view.open_merge_sheet(window, cx);
            let current = rx.try_recv().unwrap();
            deliver(
                view,
                response(
                    &old,
                    Ok(AppCommandOutput::MergeConfigYaml { yaml: "old".into() }),
                ),
                window,
                cx,
            );
            assert!(view.sheet_state.read(cx).pending_merge);
            assert_ne!(view.merge_editor.read(cx).value().as_str(), "old");
            deliver(
                view,
                response(
                    &current,
                    Ok(AppCommandOutput::MergeConfigYaml {
                        yaml: "fresh".into(),
                    }),
                ),
                window,
                cx,
            );
            assert_eq!(view.merge_editor.read(cx).value().as_str(), "fresh");
            // A duplicate response must not overwrite edits made after loading.
            view.merge_editor
                .update(cx, |editor, cx| editor.set_value("draft", window, cx));
            deliver(
                view,
                response(
                    &current,
                    Ok(AppCommandOutput::MergeConfigYaml {
                        yaml: "fresh".into(),
                    }),
                ),
                window,
                cx,
            );
            assert_eq!(view.merge_editor.read(cx).value().as_str(), "draft");
        })
    });
}

#[gpui_kit::test]
fn old_yaml_failure_cannot_cancel_new_editor(cx: &mut TestAppContext) {
    let (view, rx, cx) = setup(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            let a = ProfileId::parse("a").unwrap();
            let b = ProfileId::parse("b").unwrap();
            view.open_yaml_sheet(a, window, cx);
            let old = rx.try_recv().unwrap();
            window.close_sheet(cx);
            view.open_yaml_sheet(b.clone(), window, cx);
            let current = rx.try_recv().unwrap();
            deliver(
                view,
                response(
                    &old,
                    Err(AppError::new(ErrorCode::StorageFailed, "old failure")),
                ),
                window,
                cx,
            );
            assert_eq!(view.sheet_state.read(cx).pending_yaml, Some(b.clone()));
            deliver(
                view,
                response(
                    &current,
                    Ok(AppCommandOutput::ProfileYaml {
                        id: b,
                        yaml: "mode: rule".into(),
                    }),
                ),
                window,
                cx,
            );
            assert_eq!(view.yaml_editor.read(cx).value().as_str(), "mode: rule");
            assert!(view.sheet_state.read(cx).pending_yaml.is_none());
        })
    });
}

#[gpui_kit::test]
fn failed_merge_preview_cannot_open_an_unrelated_dialog(cx: &mut TestAppContext) {
    let (view, rx, cx) = setup(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            let id = ProfileId::parse("demo").unwrap();
            view.sheet_state.update(cx, |state, _| {
                state.request_id = Some(42);
                state.pending_merged = Some(id.clone());
                state.merge_preview_dialog = true;
            });
            view.profile_sheet_response(
                &UiResponseEnvelope {
                    request_id: Some(42),
                    operation_id: Some(42),
                    response: UiResponse::Profile {
                        request: AppCommand::PreviewMergeConfig {
                            id: id.clone(),
                            yaml: "bad".into(),
                        },
                        result: Err(AppError::new(ErrorCode::ValidationFailed, "bad merge")),
                    },
                },
                window,
                cx,
            );
            assert!(!view.sheet_state.read(cx).merge_preview_dialog);
            view.open_merged_sheet(id.clone(), window, cx);
            let request = rx.try_recv().unwrap();
            deliver(
                view,
                response(
                    &request,
                    Ok(AppCommandOutput::MergedProfileYaml {
                        id,
                        yaml: "mode: rule".into(),
                    }),
                ),
                window,
                cx,
            );
            assert!(!window.has_active_dialog(cx));
        })
    });
}

#[gpui_kit::test]
fn closed_merge_editor_ignores_pending_preview(cx: &mut TestAppContext) {
    let (view, rx, cx) = setup(cx);
    let id = ProfileId::parse("demo").unwrap();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.state.selected_profile = Some(id.clone());
            view.state
                .daemon_capabilities
                .push(crate::ipc::protocol::MERGE_PREVIEW.into());
            view.open_merge_sheet(window, cx);
            let request = rx.try_recv().unwrap();
            deliver(
                view,
                response(
                    &request,
                    Ok(AppCommandOutput::MergeConfigYaml {
                        yaml: "rules: []".into(),
                    }),
                ),
                window,
                cx,
            );
        })
    });
    cx.run_until_parked();
    let preview = cx.debug_bounds("preview-merge").unwrap();
    cx.simulate_click(preview.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    let request = rx.try_recv().unwrap();
    cx.update(|window, cx| window.dispatch_action(Box::new(Cancel), cx));
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            assert!(!window.has_active_sheet(cx));
            deliver(
                view,
                response(
                    &request,
                    Ok(AppCommandOutput::MergedProfileYaml {
                        id,
                        yaml: "mode: rule".into(),
                    }),
                ),
                window,
                cx,
            );
            assert!(!window.has_active_dialog(cx));
            assert!(!view.sheet_state.read(cx).merge_preview_dialog);
        })
    });
}

#[gpui_kit::test]
fn merged_result_rejects_keyboard_edits(cx: &mut TestAppContext) {
    let (view, rx, cx) = setup(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            let id = ProfileId::parse("demo").unwrap();
            view.open_merged_sheet(id.clone(), window, cx);
            let request = rx.try_recv().unwrap();
            deliver(
                view,
                response(
                    &request,
                    Ok(AppCommandOutput::MergedProfileYaml {
                        id,
                        yaml: "mode: rule\n".into(),
                    }),
                ),
                window,
                cx,
            );
            view.merged_editor
                .update(cx, |editor, cx| editor.focus(window, cx));
        })
    });
    cx.run_until_parked();
    cx.simulate_input("CHANGED");
    cx.run_until_parked();
    cx.update(|_, cx| {
        assert_eq!(
            view.read(cx).merged_editor.read(cx).value().as_str(),
            "mode: rule\n"
        )
    });
}

#[gpui_kit::test]
fn failed_yaml_save_keeps_sheet_and_draft_for_retry(cx: &mut TestAppContext) {
    let (view, rx, cx) = setup(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            let id = ProfileId::parse("demo").unwrap();
            view.open_yaml_sheet(id.clone(), window, cx);
            let request = rx.try_recv().unwrap();
            deliver(
                view,
                response(
                    &request,
                    Ok(AppCommandOutput::ProfileYaml {
                        id,
                        yaml: "mode: rule".into(),
                    }),
                ),
                window,
                cx,
            );
            view.yaml_editor.update(cx, |editor, cx| {
                editor.set_value("mode: [invalid", window, cx)
            });
        })
    });
    cx.run_until_parked();
    let save = cx.debug_bounds("save-yaml").unwrap();
    cx.simulate_click(save.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| window.dispatch_action(Box::new(Confirm { secondary: false }), cx));
    cx.run_until_parked();
    let request = rx.try_recv().unwrap();
    assert!(
        matches!(&request.request, UiRequest::Profile(AppCommand::UpdateProfileYaml { yaml, .. }) if yaml == "mode: [invalid")
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            deliver(
                view,
                response(
                    &request,
                    Err(AppError::new(ErrorCode::ValidationFailed, "invalid YAML")),
                ),
                window,
                cx,
            );
            assert!(window.has_active_sheet(cx));
            assert_eq!(view.yaml_editor.read(cx).value().as_str(), "mode: [invalid");
        })
    });
}
