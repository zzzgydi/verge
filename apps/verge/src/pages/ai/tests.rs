use super::{AiForm, MainView};
use crate::{
    ai::{AiCommand, AiOperation, AiSnapshot, ProviderConfig, Secret},
    ui::{Page, UiRequest, UiResponse},
};
use gpui_kit::component::Root;
use gpui_kit::{AppContext as _, TestAppContext};
use std::{cell::RefCell, rc::Rc, sync::mpsc};

fn setup(
    cx: &mut TestAppContext,
) -> (
    gpui_kit::Entity<MainView>,
    mpsc::Receiver<crate::ui::UiRequestEnvelope>,
    &mut gpui_kit::VisualTestContext,
) {
    cx.update(gpui_kit::init);
    let (tx, rx) = mpsc::sync_channel(32);
    let holder = Rc::new(RefCell::new(None));
    let slot = holder.clone();
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| MainView::new(tx, window, cx));
        view.update(cx, |view, _| {
            view.state.page = Page::Ai;
            view.state.daemon_capabilities = vec![
                crate::ai::CAPABILITY.into(),
                crate::ai::UX_CAPABILITY.into(),
            ];
            view.state.ai.config = ProviderConfig {
                base_url: "https://example.test/v1".into(),
                model: "fake".into(),
                ..Default::default()
            };
            view.state.ai.revision = 1;
        });
        *slot.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    (view, rx, cx)
}

#[gpui_kit::test]
fn ai_draft_survives_rejection_and_only_matching_ack_consumes_it(cx: &mut TestAppContext) {
    let (view, rx, cx) = setup(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.ai_form
                .prompt
                .update(cx, |input, cx| input.set_value("first draft", window, cx));
            view.state.connection_notice = Some("offline".into());
            view.send_ai(window, cx);
            assert!(rx.try_recv().is_err());
            assert_eq!(view.ai_form.prompt.read(cx).value().as_str(), "first draft");
            view.state.connection_notice = None;
            view.send_ai(window, cx);
            let request = rx.try_recv().unwrap();
            let mut request_id = request.request_id;
            assert!(matches!(
                request.request,
                UiRequest::Ai(AiCommand::Start { .. })
            ));
            assert_eq!(view.ai_form.prompt.read(cx).value().as_str(), "first draft");
            let response = UiResponse::Ai {
                operation: AiOperation::Start,
                result: Err(crate::ai::error("busy")),
            };
            view.ai_response(&response, window, cx);
            view.state
                .apply_response_envelope(crate::ui::UiResponseEnvelope {
                    request_id: Some(request_id),
                    operation_id: Some(request_id),
                    response,
                });
            assert_eq!(view.ai_form.prompt.read(cx).value().as_str(), "first draft");
            view.send_ai(window, cx);
            request_id = rx.try_recv().unwrap().request_id;
            view.ai_form
                .prompt
                .update(cx, |input, cx| input.set_value("next draft", window, cx));
            let snapshot = AiSnapshot {
                revision: 2,
                ..view.state.ai.clone()
            };
            let response = UiResponse::Ai {
                operation: AiOperation::Start,
                result: Ok(snapshot),
            };
            view.ai_response(&response, window, cx);
            view.state
                .apply_response_envelope(crate::ui::UiResponseEnvelope {
                    request_id: Some(request_id),
                    operation_id: Some(request_id),
                    response,
                });
            assert_eq!(view.ai_form.prompt.read(cx).value().as_str(), "next draft");
            view.send_ai(window, cx);
            rx.try_recv().unwrap();
            let response = UiResponse::Ai {
                operation: AiOperation::Start,
                result: Ok(AiSnapshot {
                    revision: 3,
                    ..view.state.ai.clone()
                }),
            };
            view.ai_response(&response, window, cx);
            assert!(view.ai_form.prompt.read(cx).value().is_empty());
        })
    });
}

#[gpui_kit::test]
fn ai_save_tests_current_draft_only_after_success_and_retains_failed_key(cx: &mut TestAppContext) {
    let (view, rx, cx) = setup(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.ai_form.settings_open = true;
            view.ai_form.base.update(cx, |input, cx| {
                input.set_value("https://new.test/v1", window, cx)
            });
            view.ai_form
                .model
                .update(cx, |input, cx| input.set_value("new-model", window, cx));
            view.ai_form
                .key
                .update(cx, |input, cx| input.set_value("new-secret", window, cx));
            cx.notify();
        })
    });
    cx.run_until_parked();
    let save = cx.debug_bounds("ai-save").unwrap();
    cx.simulate_click(save.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    let envelope = rx.try_recv().unwrap();
    let UiRequest::Ai(AiCommand::SaveConfig {
        config, api_key, ..
    }) = envelope.request
    else {
        panic!("must save current form")
    };
    assert_eq!(config.model, "new-model");
    assert_eq!(api_key, Secret("new-secret".into()));
    assert!(rx.try_recv().is_err());
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            let response = UiResponse::Ai {
                operation: AiOperation::Save,
                result: Ok(AiSnapshot {
                    revision: 2,
                    run_id: 2,
                    busy: true,
                    operation: Some(AiOperation::Save),
                    ..view.state.ai.clone()
                }),
            };
            view.ai_response(&response, window, cx);
            view.state.apply_response(response);
            view.finish_ai_save(window, cx);
            assert_eq!(view.ai_form.key.read(cx).value().as_str(), "new-secret");
            assert!(rx.try_recv().is_err());
            view.state.ai.busy = false;
            view.state.ai.error = Some("Keychain failed".into());
            view.finish_ai_save(window, cx);
            assert_eq!(view.ai_form.key.read(cx).value().as_str(), "new-secret");
            assert!(rx.try_recv().is_err());
            view.ai_form.save_requested = true;
            let response = UiResponse::Ai {
                operation: AiOperation::Save,
                result: Ok(AiSnapshot {
                    revision: 3,
                    run_id: 3,
                    busy: false,
                    error: None,
                    config: config.clone(),
                    operation: Some(AiOperation::Save),
                    ..view.state.ai.clone()
                }),
            };
            view.ai_response(&response, window, cx);
            view.state.apply_response(response);
            view.finish_ai_save(window, cx);
            assert!(view.ai_form.key.read(cx).value().is_empty());
            assert!(matches!(
                rx.try_recv().unwrap().request,
                UiRequest::Ai(AiCommand::TestProvider)
            ));
            assert_eq!(view.state.ai.config, config);
        })
    });
}

#[gpui_kit::test]
fn ai_keyboard_newline_and_send_use_same_draft_rules(cx: &mut TestAppContext) {
    let (view, rx, cx) = setup(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.ai_form.prompt.update(cx, |input, cx| {
                input.set_value("问题", window, cx);
                input.focus(window, cx);
            });
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("shift-enter");
    cx.run_until_parked();
    assert!(rx.try_recv().is_err());
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            assert!(view.ai_form.prompt.read(cx).value().contains('\n'))
        })
    });
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(matches!(
        rx.try_recv().unwrap().request,
        UiRequest::Ai(AiCommand::Start { .. })
    ));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(rx.try_recv().is_err());
}

#[gpui_kit::test]
fn ai_background_snapshots_do_not_replace_provider_drafts(cx: &mut TestAppContext) {
    let (view, _, cx) = setup(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.ai_form.sync(&view.state.ai, window, cx);
            view.ai_form
                .model
                .update(cx, |input, cx| input.set_value("unsaved model", window, cx));
            view.ai_form.reset_sync();
            view.state.ai.config.model = "remote model".into();
            view.ai_form.sync(&view.state.ai, window, cx);
            assert_eq!(
                view.ai_form.model.read(cx).value().as_str(),
                "unsaved model"
            );
            let mut fresh = AiForm::new(window, cx);
            fresh.provider_dirty = true;
            fresh.model.update(cx, |input, cx| {
                input.set_value("typed before first snapshot", window, cx)
            });
            fresh.sync(&view.state.ai, window, cx);
            assert_eq!(
                fresh.model.read(cx).value().as_str(),
                "typed before first snapshot"
            );
        })
    });
}

#[gpui_kit::test]
fn ai_ime_confirmation_does_not_send_a_question(cx: &mut TestAppContext) {
    use gpui_kit::EntityInputHandler as _;
    use gpui_kit::component::input::InputEvent;
    let (view, rx, cx) = setup(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.ai_form.prompt.update(cx, |input, cx| {
                input.replace_and_mark_text_in_range(None, "中文", Some(2..2), window, cx);
                cx.emit(InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                });
            });
        })
    });
    cx.run_until_parked();
    assert!(rx.try_recv().is_err());
}

#[gpui_kit::test]
fn ai_settings_actions_stay_visible_with_advanced_fields_at_minimum_size(cx: &mut TestAppContext) {
    let (view, _rx, cx) = setup(cx);
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(960.), gpui_kit::px(640.)));
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.ai_form.settings_open = true;
            view.ai_form.advanced_open = true;
            view.state.ai.has_key = true;
            cx.notify();
        })
    });
    cx.run_until_parked();
    let save = cx.debug_bounds("ai-save").unwrap();
    assert!(save.bottom() < gpui_kit::px(640.));
    assert!(save.top() > gpui_kit::px(0.));
}

#[gpui_kit::test]
fn ai_stream_follows_bottom_but_does_not_pull_reader_from_older_content(cx: &mut TestAppContext) {
    let (view, _rx, cx) = setup(cx);
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(960.), gpui_kit::px(640.)));
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.state.ai.messages = vec![crate::ai::ChatMessage {
                role: "assistant".into(),
                text: "A paragraph of diagnostic information.\n\n".repeat(40),
                evidence: vec![],
            }];
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            assert!(view.ai_form.scroll.max_offset().y > gpui_kit::px(100.));
            view.ai_form
                .scroll
                .set_offset(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-80.)));
            let mut state = view.state.ai.clone();
            state.revision += 1;
            state.messages[0].text.push_str("More text.\n\n");
            let response = UiResponse::Ai {
                operation: AiOperation::State,
                result: Ok(state),
            };
            view.ai_response(&response, window, cx);
            view.state.apply_response(response);
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            assert_eq!(view.ai_form.scroll.offset().y, gpui_kit::px(-80.));
            view.ai_form.scroll.scroll_to_bottom();
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            assert!(view.ai_form.at_bottom());
            let mut state = view.state.ai.clone();
            state.revision += 1;
            state.messages[0].text.push_str("More streaming text.\n\n");
            let response = UiResponse::Ai {
                operation: AiOperation::State,
                result: Ok(state),
            };
            view.ai_response(&response, window, cx);
            view.state.apply_response(response);
            cx.notify();
        })
    });
    cx.run_until_parked();
    cx.update(|_, cx| view.update(cx, |view, _| assert!(view.ai_form.at_bottom())));
}
