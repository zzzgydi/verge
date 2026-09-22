use crate::{
    domain::{AppCommand, AppCommandOutput, ProfileId},
    i18n::tr,
    ui::{UiAction, UiResponse, UiResponseEnvelope},
    view::MainView,
};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Textarea, TextareaState},
    menu::{DropdownMenu as _, PopupMenuItem},
    notification::Notification,
    v_flex,
};
use gpui_kit::{prelude::FluentBuilder as _, *};

pub(crate) struct ScriptEditor {
    scope: Option<ProfileId>,
    profile: Option<ProfileId>,
    source: Entity<TextareaState>,
    request: Option<u64>,
    loaded: bool,
    enabled: bool,
    error: Option<String>,
    connected: bool,
}

impl MainView {
    pub fn open_script_sheet(
        &mut self,
        scope: Option<ProfileId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sheet_state
            .update(cx, |state, _| *state = Default::default());
        // A new entity for every open prevents stale replies from touching another draft.
        let lang = self.lang();
        let profile = scope
            .clone()
            .or_else(|| self.state.selected_profile.clone())
            .or_else(|| self.state.profiles.first().map(|p| p.id.clone()));
        let source = cx.new(|cx| TextareaState::new(window, cx));
        let editor = cx.new(|_| ScriptEditor {
            scope: scope.clone(),
            profile,
            source,
            request: None,
            loaded: false,
            enabled: false,
            error: None,
            connected: true,
        });
        self.script_editor = Some(editor.clone());
        let profiles = self.state.profiles.clone();
        let view = cx.entity();
        let command = AppCommand::GetProfileScript { id: scope.clone() };
        let title = match scope
            .as_ref()
            .and_then(|id| profiles.iter().find(|p| &p.id == id))
        {
            Some(profile) => format!("{} · {}", tr(lang, "profiles.script"), profile.name),
            None => tr(lang, "profiles.global_script").to_owned(),
        };
        let sheet_editor = editor.clone();
        window.open_sheet(cx, move |sheet, _, cx| {
            let editor = sheet_editor.clone();
            let state = editor.read(cx);
            let source = state.source.clone();
            let busy = state.request.is_some() || !state.connected;
            let loaded = state.loaded;
            let enabled = state.enabled;
            let has_profile = state.profile.is_some();
            let name = state
                .profile
                .as_ref()
                .and_then(|id| profiles.iter().find(|p| &p.id == id))
                .map(|p| p.name.as_str())
                .unwrap_or("—");
            let error = state.error.clone();
            sheet
                .title(title.clone())
                .size(rems(38.))
                .on_close({
                    let view = view.clone();
                    move |_, _, cx| {
                        view.update(cx, |view, _| view.script_editor = None);
                    }
                })
                .child(
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr(lang, "script.desc")),
                        )
                        .child(div().text_xs().child(tr(
                            lang,
                            if enabled {
                                "script.enabled"
                            } else {
                                "script.disabled"
                            },
                        )))
                        .child(
                            Button::new("script-preview-profile")
                                .small()
                                .outline()
                                .disabled(busy || scope.is_some())
                                .label(format!("{}{name}", tr(lang, "script.preview_target")))
                                .dropdown_menu({
                                    let profiles = profiles.clone();
                                    let editor = editor.clone();
                                    move |mut menu, _, _| {
                                        for profile in &profiles {
                                            let id = profile.id.clone();
                                            let editor = editor.clone();
                                            menu = menu.item(
                                                PopupMenuItem::new(profile.name.clone()).on_click(
                                                    move |_, _, cx| {
                                                        editor.update(cx, |state, cx| {
                                                            state.profile = Some(id.clone());
                                                            cx.notify();
                                                        });
                                                    },
                                                ),
                                            );
                                        }
                                        menu
                                    }
                                }),
                        )
                        .when(!has_profile, |col| {
                            col.child(div().text_xs().child(tr(lang, "script.no_profile")))
                        })
                        .when_some(error, |col, error| {
                            col.child(div().text_xs().text_color(cx.theme().danger).child(error))
                        })
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .font_family(cx.theme().mono_font_family.clone())
                                .child(Textarea::new(&source).readonly(!loaded || busy).h_full()),
                        ),
                )
                .footer(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .justify_end()
                        .child(
                            Button::new("disable-script")
                                .label(tr(lang, "script.disable"))
                                .ghost()
                                .disabled(!loaded || busy || !enabled)
                                .on_click({
                                    let view = view.clone();
                                    move |_, _, cx| {
                                        view.update(cx, |view, cx| {
                                            view.submit_script("disable", cx)
                                        });
                                    }
                                }),
                        )
                        .child(
                            Button::new("preview-script")
                                .debug_selector(|| "preview-script".into())
                                .label(tr(lang, "script.preview"))
                                .outline()
                                .disabled(!loaded || busy || !has_profile)
                                .on_click({
                                    let view = view.clone();
                                    move |_, _, cx| {
                                        view.update(cx, |view, cx| {
                                            view.submit_script("preview", cx)
                                        });
                                    }
                                }),
                        )
                        .child(
                            Button::new("save-script-draft")
                                .debug_selector(|| "save-script-draft".into())
                                .label(tr(lang, "script.save"))
                                .outline()
                                .disabled(!loaded || busy)
                                .on_click({
                                    let view = view.clone();
                                    move |_, _, cx| {
                                        view.update(cx, |view, cx| view.submit_script("save", cx));
                                    }
                                }),
                        )
                        .child(
                            Button::new("enable-script")
                                .debug_selector(|| "enable-script".into())
                                .label(tr(lang, "script.enable"))
                                .primary()
                                .disabled(!loaded || busy || !has_profile)
                                .on_click({
                                    let view = view.clone();
                                    move |_, _, cx| {
                                        view.update(cx, |view, cx| {
                                            view.submit_script("enable", cx)
                                        });
                                    }
                                }),
                        ),
                )
        });
        let request = self.dispatch_sheet_request(UiAction::ProfileScript(command), cx);
        editor.update(cx, |state, cx| {
            state.request = request;
            if request.is_none() {
                state.error = Some(tr(lang, "connection.incompatible").into());
            }
            cx.notify();
        });
    }

    pub(crate) fn script_disconnected(&mut self, cx: &mut Context<Self>) {
        if let Some(editor) = &self.script_editor {
            let lang = self.lang();
            editor.update(cx, |state, cx| {
                state.request = None;
                state.connected = false;
                state.error = Some(tr(lang, "connection.reconnecting").into());
                cx.notify();
            });
        }
    }

    pub(crate) fn script_reconnected(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.script_editor.clone() else {
            return;
        };
        let id = editor.read(cx).scope.clone();
        let request = self.dispatch_sheet_request(
            UiAction::ProfileScript(AppCommand::GetProfileScript { id }),
            cx,
        );
        editor.update(cx, |state, cx| {
            state.connected = true;
            state.request = request;
            cx.notify();
        });
    }

    pub(crate) fn submit_script(&mut self, operation: &str, cx: &mut Context<Self>) {
        let Some(editor) = self.script_editor.clone() else {
            return;
        };
        let state = editor.read(cx);
        if state.request.is_some() || !state.loaded || !state.connected {
            return;
        }
        let id = state.scope.clone();
        let source = state.source.read(cx).value().to_string();
        let command = match operation {
            "save" => AppCommand::SaveScriptDraft { id, source },
            "enable" => AppCommand::SetProfileScript {
                id,
                source: Some(source),
            },
            "disable" => AppCommand::SetProfileScript { id, source: None },
            _ => {
                let Some(profile) = state.profile.clone() else {
                    return;
                };
                AppCommand::PreviewProfileScript {
                    id,
                    profile,
                    source,
                }
            }
        };
        let request = self.dispatch_sheet_request(UiAction::ProfileScript(command), cx);
        let lang = self.lang();
        editor.update(cx, |state, cx| {
            state.request = request;
            state.error = request
                .is_none()
                .then(|| tr(lang, "connection.incompatible").into());
            cx.notify();
        });
    }

    pub(crate) fn script_response(
        &mut self,
        envelope: &UiResponseEnvelope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(editor) = self.script_editor.clone() else {
            return;
        };
        if envelope.request_id.is_none() || envelope.request_id != editor.read(cx).request {
            return;
        }
        let UiResponse::Profile { request, result } = &envelope.response else {
            return;
        };
        let lang = self.lang();
        editor.update(cx, |state, cx| {
            state.request = None;
            match result {
                Err(error) => state.error = Some(error.message.clone()),
                Ok(result) => {
                    state.error = None;
                    match &result.output {
                        AppCommandOutput::ProfileScript(script) => {
                            state.enabled = script.active.is_some();
                            if matches!(request, AppCommand::GetProfileScript { .. })
                                && !state.loaded
                            {
                                state.source.update(cx, |input, cx| {
                                    input.set_value(
                                        if script.draft.is_empty() {
                                            crate::script::TEMPLATE.to_owned()
                                        } else {
                                            script.draft.clone()
                                        },
                                        window,
                                        cx,
                                    )
                                });
                            } else if !matches!(request, AppCommand::GetProfileScript { .. }) {
                                window.push_notification(
                                    Notification::success(tr(
                                        lang,
                                        if matches!(request, AppCommand::SaveScriptDraft { .. }) {
                                            "script.saved"
                                        } else {
                                            "script.applied"
                                        },
                                    )),
                                    cx,
                                );
                            }
                            state.loaded = true;
                        }
                        AppCommandOutput::ScriptPreview(preview) => {
                            let result = cx.new(|cx| {
                                TextareaState::new(window, cx).default_value(preview.yaml.clone())
                            });
                            let logs = preview.logs.join("\n");
                            let log_editor =
                                cx.new(|cx| TextareaState::new(window, cx).default_value(logs));
                            let has_logs = !preview.logs.is_empty();
                            window.open_dialog(cx, move |dialog, window, _| {
                                dialog
                                    .title(tr(lang, "script.preview"))
                                    .width(rems(36.).to_pixels(window.rem_size()))
                                    .child(
                                        div()
                                            .h(px(if has_logs { 220. } else { 340. }))
                                            .child(Textarea::new(&result).readonly(true).h_full()),
                                    )
                                    .when(has_logs, |dialog| {
                                        dialog.child(div().h(px(100.)).child(
                                            Textarea::new(&log_editor).readonly(true).h_full(),
                                        ))
                                    })
                            });
                        }
                        _ => {}
                    }
                }
            }
            cx.notify();
        });
    }
}
