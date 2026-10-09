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
                crate::ai::ACTIONS_CAPABILITY.into(),
            ];
            view.state.ai.config = ProviderConfig {
                base_url: "https://example.test/v1".into(),
                model: "fake".into(),
                ..Default::default()
            };
            view.state.ai.revision = 1;
            view.state.application_settings = Some(crate::domain::ApplicationSettingsSnapshot {
                settings: Default::default(),
                data_directory: "/tmp/verge-test".into(),
                app_version: None,
            });
        });
        *slot.borrow_mut() = Some(view.clone());
        Root::new(view, window, cx)
    });
    let view = holder.borrow().clone().unwrap();
    (view, rx, cx)
}

#[gpui_kit::test]
fn proposal_requires_two_clicks_and_sends_only_bound_confirmation(cx: &mut TestAppContext) {
    let (view, rx, cx) = setup(cx);
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.state.ai.proposals = vec![crate::ai::proposals::Proposal {
                id: "proposal-1".into(),
                run_id: 1,
                digest: "bound-digest".into(),
                kind: "mode".into(),
                target: "Direct".into(),
                expires_at: crate::ai::proposals::now() + 300,
                status: "pending".into(),
                changes: vec![crate::ai::proposals::Change {
                    path: "mode".into(),
                    before: "Rule".into(),
                    after: "Direct".into(),
                }],
                evidence_id: "R1-E8".into(),
                result: None,
            }];
            cx.notify();
        })
    });
    cx.run_until_parked();
    let bounds = cx.debug_bounds("ai-proposal-apply-0").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(rx.try_recv().is_err());
    let cancel = cx.debug_bounds("ai-proposal-cancel-0").unwrap();
    cx.simulate_click(cancel.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    cx.update(|_, cx| assert!(view.read(cx).ai_form.confirm_proposal.is_none()));
    assert!(rx.try_recv().is_err());
    let bounds = cx.debug_bounds("ai-proposal-apply-0").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    let bounds = cx.debug_bounds("ai-proposal-apply-0").unwrap();
    cx.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(
        matches!(rx.try_recv().unwrap().request,UiRequest::Ai(AiCommand::Approve {id,digest}) if id=="proposal-1" && digest=="bound-digest")
    );
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
            view.state.page = Page::Settings;
            view.settings_category = crate::pages::settings::SettingsCategory::Ai;
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
    let save = cx.debug_bounds("ai-test").unwrap();
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
            view.state.ai.error = Some("Cannot save AI settings".into());
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
            view.state.page = Page::Settings;
            view.settings_category = crate::pages::settings::SettingsCategory::Ai;
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
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            let mut state = view.state.ai.clone();
            state.revision += 1;
            state.operation = Some(AiOperation::Start);
            state.error = Some("AI request timed out".into());
            let response = UiResponse::Ai {
                operation: AiOperation::State,
                result: Ok(state),
            };
            view.ai_response(&response, window, cx);
            view.state.apply_response(response);
            cx.notify();
        });
    });
    cx.run_until_parked();
    let error = cx.debug_bounds("ai-chat-error").unwrap();
    assert!(error.bottom() <= cx.debug_bounds("ai-conversation").unwrap().bottom());
    cx.update(|_, cx| view.update(cx, |view, _| assert!(view.ai_form.at_bottom())));
}

#[gpui_kit::test]
fn ai_provider_entry_opens_settings_and_preserves_chat_and_provider_drafts(
    cx: &mut TestAppContext,
) {
    let (view, rx, cx) = setup(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.ai_form.prompt.update(cx, |input, cx| {
                input.set_value("unfinished question", window, cx)
            });
            view.ai_form
                .key
                .update(cx, |input, cx| input.set_value("unsaved-key", window, cx));
        })
    });
    cx.run_until_parked();
    let entry = cx.debug_bounds("ai-settings").unwrap();
    cx.simulate_click(entry.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    cx.update(|_, cx| {
        let view = view.read(cx);
        assert_eq!(view.state.page, Page::Settings);
        assert!(view.settings_category == crate::pages::settings::SettingsCategory::Ai);
        assert_eq!(
            view.ai_form.prompt.read(cx).value().as_str(),
            "unfinished question"
        );
        assert_eq!(view.ai_form.key.read(cx).value().as_str(), "unsaved-key");
    });
    assert!(cx.debug_bounds("ai-save").is_some());
    assert!(
        rx.try_iter()
            .any(|r| matches!(r.request, UiRequest::Ai(AiCommand::GetState)))
    );
    let general = cx
        .debug_bounds("settings-category-settings.group.general")
        .unwrap();
    cx.simulate_click(general.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("ai-save").is_none());
    let ai_tab = cx.debug_bounds("settings-category-ai.title").unwrap();
    cx.simulate_click(ai_tab.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(cx.debug_bounds("ai-save").is_some());
    cx.update(|_, cx| {
        assert_eq!(
            view.read(cx).ai_form.key.read(cx).value().as_str(),
            "unsaved-key"
        )
    });
}

#[gpui_kit::test]
fn unconfigured_model_opens_settings_before_sending(cx: &mut TestAppContext) {
    let (view, rx, cx) = setup(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.state.ai.config = crate::ai::ProviderConfig::default();
            view.ai_form
                .prompt
                .update(cx, |input, cx| input.set_value("hello", window, cx));
            view.send_ai(window, cx);
            cx.notify();
        })
    });
    cx.run_until_parked();
    assert!(rx.try_recv().is_err());
    let entry = cx.debug_bounds("ai-connect").unwrap();
    cx.simulate_click(entry.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    cx.update(|_, cx| assert_eq!(view.read(cx).state.page, Page::Settings));
    assert!(cx.debug_bounds("ai-save").is_some());
}

#[gpui_kit::test]
fn saving_does_not_start_a_provider_request_and_normalizes_the_form(cx: &mut TestAppContext) {
    let (view, rx, cx) = setup(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.ai_form.sync(&view.state.ai, window, cx);
            view.ai_form.base.update(cx, |input, cx| {
                input.set_value(" https://example.test/v1/ ", window, cx)
            });
            view.ai_form
                .key
                .update(cx, |input, cx| input.set_value("new-test-key", window, cx));
            view.save_ai_settings(false, cx);
            let request = rx.try_recv().unwrap();
            let UiRequest::Ai(AiCommand::SaveConfig { config, .. }) = request.request else {
                panic!()
            };
            assert_eq!(config.base_url, "https://example.test/v1");
            let response = UiResponse::Ai {
                operation: AiOperation::Save,
                result: Ok(AiSnapshot {
                    revision: 2,
                    run_id: 2,
                    busy: false,
                    operation: Some(AiOperation::Save),
                    config,
                    has_key: true,
                    ..view.state.ai.clone()
                }),
            };
            view.ai_response(&response, window, cx);
            view.state.apply_response(response);
            view.finish_ai_save(window, cx);
            assert!(rx.try_recv().is_err());
            assert!(!view.ai_form.has_provider_draft(cx));
            assert!(view.ai_form.key.read(cx).value().is_empty());
            assert_eq!(
                view.ai_form.base.read(cx).value().as_str(),
                "https://example.test/v1"
            );
        })
    });
}

#[gpui_kit::test]
fn clean_provider_form_tracks_new_settings_but_keeps_unsaved_edits(cx: &mut TestAppContext) {
    let (view, _, cx) = setup(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.ai_form.sync(&view.state.ai, window, cx);
            assert!(!view.ai_form.has_provider_draft(cx));
            view.state.ai.config.model = "new-server-model".into();
            view.ai_form.sync(&view.state.ai, window, cx);
            assert_eq!(
                view.ai_form.model.read(cx).value().as_str(),
                "new-server-model"
            );
            view.ai_form
                .model
                .update(cx, |input, cx| input.set_value("user-draft", window, cx));
            view.state.ai.config.model = "third-model".into();
            view.ai_form.sync(&view.state.ai, window, cx);
            assert_eq!(view.ai_form.model.read(cx).value().as_str(), "user-draft");
        })
    });
}

#[gpui_kit::test]
fn clear_key_and_replacement_are_mutually_exclusive(cx: &mut TestAppContext) {
    let (view, _, cx) = setup(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.state.page = Page::Settings;
            view.settings_category = crate::pages::settings::SettingsCategory::Ai;
            view.state.ai.has_key = true;
            view.ai_form.sync(&view.state.ai, window, cx);
            view.ai_form
                .key
                .update(cx, |input, cx| input.set_value("replacement", window, cx));
            cx.notify();
        })
    });
    cx.run_until_parked();
    let clear = cx.debug_bounds("ai-clear-key").unwrap();
    cx.simulate_click(clear.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            assert!(view.ai_form.clear_key);
            assert!(view.ai_form.key.read(cx).value().is_empty());
            view.ai_form
                .key
                .update(cx, |input, cx| input.focus(window, cx));
        })
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("a");
    cx.run_until_parked();
    cx.update(|_, cx| assert!(!view.read(cx).ai_form.clear_key));
}

#[gpui_kit::test]
fn scope_popover_does_not_move_composer_and_stop_is_not_duplicated(cx: &mut TestAppContext) {
    let (view, rx, cx) = setup(cx);
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(960.), gpui_kit::px(640.)));
    cx.run_until_parked();
    let composer = cx.debug_bounds("ai-composer").unwrap();
    let scope = cx.debug_bounds("ai-data-scope").unwrap();
    assert!(scope.bottom() < cx.debug_bounds("ai-conversation").unwrap().top());
    cx.simulate_click(scope.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert_eq!(cx.debug_bounds("ai-composer").unwrap(), composer);
    let body = cx.debug_bounds("ai-scope-body").unwrap();
    assert!(body.size.height > gpui_kit::px(80.));
    assert!(body.bottom() < composer.top());
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.state.ai.busy = true;
            view.state.ai.operation = Some(AiOperation::Start);
            view.stop_ai(cx);
            view.stop_ai(cx);
            assert!(matches!(
                rx.try_recv().unwrap().request,
                UiRequest::Ai(AiCommand::Cancel)
            ));
            assert!(rx.try_recv().is_err());
            let response = UiResponse::Ai {
                operation: AiOperation::State,
                result: Ok(AiSnapshot {
                    revision: 2,
                    busy: false,
                    ..view.state.ai.clone()
                }),
            };
            view.ai_response(&response, window, cx);
            assert!(!view.ai_form.stop_requested);
        })
    });
}

#[gpui_kit::test]
fn settings_errors_stay_out_of_chat_and_new_chat_clears_chat_feedback(cx: &mut TestAppContext) {
    let (view, _, cx) = setup(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            let failed = UiResponse::Ai {
                operation: AiOperation::Save,
                result: Err(crate::ai::error("Invalid AI key")),
            };
            view.ai_response(&failed, window, cx);
            view.state.apply_response(failed);
            assert!(view.state.last_error.is_none());
            assert!(view.ai_form.local_error.is_none());
            assert!(view.ai_form.settings_error.is_some());
            view.ai_form.local_error = Some("old chat error".into());
            view.ai_form.copied = Some(0);
            view.ai_form.confirm_clear = true;
            view.ai_form.prompt.update(cx, |input, cx| {
                input.set_value("retained draft", window, cx)
            });
            let cleared = UiResponse::Ai {
                operation: AiOperation::Clear,
                result: Ok(AiSnapshot {
                    revision: 2,
                    ..Default::default()
                }),
            };
            view.ai_response(&cleared, window, cx);
            assert!(view.ai_form.local_error.is_none());
            assert!(view.ai_form.copied.is_none());
            assert!(!view.ai_form.confirm_clear);
            assert_eq!(
                view.ai_form.prompt.read(cx).value().as_str(),
                "retained draft"
            );
        })
    });
}

#[gpui_kit::test]
fn new_chat_confirmation_is_visible_when_reading_the_end_of_a_long_chat(cx: &mut TestAppContext) {
    let (view, rx, cx) = setup(cx);
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(960.), gpui_kit::px(640.)));
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.state.ai.messages = vec![crate::ai::ChatMessage {
                role: "assistant".into(),
                text: "Long answer\n\n".repeat(80),
                evidence: vec![],
            }];
            view.ai_form.scroll.scroll_to_bottom();
            cx.notify();
        });
    });
    cx.run_until_parked();
    let clear = cx.debug_bounds("ai-clear").unwrap();
    cx.simulate_click(clear.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    let confirm = cx.debug_bounds("ai-clear-confirm").unwrap();
    assert!(confirm.top() > gpui_kit::px(0.));
    assert!(confirm.bottom() < cx.debug_bounds("ai-conversation").unwrap().top());
    cx.simulate_click(confirm.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(matches!(
        rx.try_recv().unwrap().request,
        UiRequest::Ai(AiCommand::Clear)
    ));
}

#[gpui_kit::test]
fn initial_load_error_can_be_retried_from_settings(cx: &mut TestAppContext) {
    let (view, rx, cx) = setup(cx);
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(960.), gpui_kit::px(640.)));
    cx.update(|_, cx| {
        view.update(cx, |view, cx| {
            view.state.page = Page::Settings;
            view.settings_category = crate::pages::settings::SettingsCategory::Ai;
            view.ai_form.advanced_open = true;
            view.state.ai.operation = Some(AiOperation::State);
            view.state.ai.error = Some("Unable to load AI settings".into());
            view.ai_form.settings_error = Some("Previous error".into());
            cx.notify();
        });
    });
    cx.run_until_parked();
    let reload = cx.debug_bounds("ai-reload-settings").unwrap();
    assert!(reload.top() > gpui_kit::px(0.));
    assert!(reload.bottom() < cx.debug_bounds("ai-save").unwrap().top());
    let error = cx.debug_bounds("ai-settings-error").unwrap();
    assert!(error.top() > gpui_kit::px(0.));
    assert!(error.bottom() < reload.top());
    cx.simulate_click(reload.center(), gpui_kit::Modifiers::default());
    cx.run_until_parked();
    assert!(matches!(
        rx.try_recv().unwrap().request,
        UiRequest::Ai(AiCommand::GetState)
    ));
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.ai_response(
                &UiResponse::Ai {
                    operation: AiOperation::State,
                    result: Ok(AiSnapshot {
                        revision: 2,
                        ..Default::default()
                    }),
                },
                window,
                cx,
            );
            assert!(view.ai_form.settings_error.is_none());
        });
    });
}

#[gpui_kit::test]
fn provider_fields_and_advanced_action_fit_the_small_window(cx: &mut TestAppContext) {
    let (view, _, cx) = setup(cx);
    cx.simulate_resize(gpui_kit::size(gpui_kit::px(960.), gpui_kit::px(640.)));
    for language in ["en", "zh-CN"] {
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.state.page = Page::Settings;
                view.settings_category = crate::pages::settings::SettingsCategory::Ai;
                view.state
                    .application_settings
                    .as_mut()
                    .unwrap()
                    .settings
                    .language = language.into();
                view.state.ai.has_key = true;
                view.sync_form_inputs(window, cx);
                cx.notify();
            })
        });
        cx.run_until_parked();
        let footer = cx.debug_bounds("ai-save").unwrap();
        for selector in ["ai-key-hint", "ai-clear-key", "ai-advanced"] {
            let control = cx.debug_bounds(selector).unwrap();
            assert!(control.size.height > gpui_kit::px(0.));
            assert!(
                control.bottom() < footer.top(),
                "{language}: {selector} is clipped"
            );
        }
    }
}
