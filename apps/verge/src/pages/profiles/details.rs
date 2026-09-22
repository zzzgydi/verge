use crate::{
    domain::{AppCommand, AppError, ErrorCode, Profile, ProfileSource, UpdatePolicy},
    i18n::tr,
    ui::{UiAction, UiResponse, UiResponseEnvelope},
    view::MainView,
};
use gpui_kit::component::{
    Disableable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::DialogButtonProps,
    form::{field, v_form},
    h_flex,
    input::{Input, InputState},
    notification::Notification,
};
use gpui_kit::{prelude::FluentBuilder as _, *};

type SaveDetails = std::rc::Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) -> bool>;

impl MainView {
    pub fn open_profile_details(
        &mut self,
        profile: Profile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.details_request.update(cx, |request, cx| {
            *request = None;
            cx.notify();
        });
        let lang = self.lang();
        let remote = matches!(profile.source, ProfileSource::Remote { .. });
        let name = cx.new(|cx| InputState::new(window, cx).default_value(profile.name.clone()));
        let url = cx.new(|cx| {
            InputState::new(window, cx).default_value(match &profile.source {
                ProfileSource::Remote { url } => url.clone(),
                _ => String::new(),
            })
        });
        let interval = cx.new(|cx| {
            InputState::new(window, cx).default_value(match profile.update_policy {
                UpdatePolicy::Interval { seconds } => seconds.to_string(),
                _ => String::new(),
            })
        });
        let ua = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(profile.user_agent.clone().unwrap_or_default())
        });
        let view = cx.entity();
        let request = self.details_request.clone();
        let observation = std::rc::Rc::new(cx.observe(&request, |_, _, cx| cx.notify()));
        window.open_dialog(cx, move |dialog, window, cx| {
            let _observation = &observation;
            let busy = request.read(cx).is_some();
            let name = name.clone();
            let url = url.clone();
            let interval = interval.clone();
            let ua = ua.clone();
            let view = view.clone();
            let id = profile.id.clone();
            let save: SaveDetails = {
                let name = name.clone();
                let url = url.clone();
                let interval = interval.clone();
                let ua = ua.clone();
                let view = view.clone();
                std::rc::Rc::new(move |_, window, cx| {
                    let result = (|| -> Result<(), AppError> {
                        let value = interval.read(cx).value().trim().to_owned();
                        let update_policy =
                            if !remote || value.is_empty() {
                                UpdatePolicy::Manual
                            } else {
                                let seconds =
                                    value.parse::<u64>().ok().filter(|s| *s > 0).ok_or_else(
                                        || {
                                            AppError::new(
                                                ErrorCode::InvalidInput,
                                                tr(lang, "profile.interval_optional"),
                                            )
                                        },
                                    )?;
                                UpdatePolicy::Interval { seconds }
                            };
                        let name = name.read(cx).value().to_string();
                        let source = if remote {
                            ProfileSource::Remote {
                                url: url.read(cx).value().trim().to_owned(),
                            }
                        } else {
                            ProfileSource::Local
                        };
                        let agent = ua.read(cx).value().trim().to_owned();
                        let user_agent = (remote && !agent.is_empty()).then_some(agent);
                        Profile::new(
                            id.clone(),
                            &name,
                            source.clone(),
                            update_policy.clone(),
                            0,
                            user_agent.clone(),
                        )?;
                        view.update(cx, |view, cx| {
                            if view.details_request.read(cx).is_none() {
                                let pending = view.dispatch_sheet_request(
                                    UiAction::UpdateProfileDetails {
                                        id: id.clone(),
                                        name,
                                        source,
                                        update_policy,
                                        user_agent,
                                    },
                                    cx,
                                );
                                view.details_request.update(cx, |request, cx| {
                                    *request = pending;
                                    cx.notify();
                                });
                            }
                        });
                        Ok(())
                    })();
                    if let Err(error) = result {
                        window.push_notification(Notification::error(error.message), cx);
                    }
                    false
                })
            };
            let save_click = save.clone();
            dialog
                .title(tr(lang, "profiles.edit_details"))
                .width(rems(32.).to_pixels(window.rem_size()))
                .child(
                    v_form()
                        .child(
                            field()
                                .label(tr(lang, "dialog.import.name"))
                                .required(true)
                                .description(tr(
                                    lang,
                                    if remote {
                                        "profile.details.desc"
                                    } else {
                                        "profile.local_edit_desc"
                                    },
                                ))
                                .child(
                                    div()
                                        .debug_selector(|| "profile-details-name".into())
                                        .child(Input::new(&name).readonly(busy)),
                                ),
                        )
                        .when(remote, |form| {
                            form.child(
                                field()
                                    .label(tr(lang, "dialog.import.url"))
                                    .required(true)
                                    .child(Input::new(&url).readonly(busy)),
                            )
                            .child(
                                field()
                                    .label(tr(lang, "dialog.import.interval"))
                                    .description(tr(lang, "profile.interval_optional"))
                                    .child(Input::new(&interval).readonly(busy)),
                            )
                            .child(
                                field()
                                    .label("User-Agent")
                                    .child(Input::new(&ua).readonly(busy)),
                            )
                        }),
                )
                .footer(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("profile-details-cancel")
                                .label(tr(lang, "common.cancel"))
                                .on_click({
                                    let view = view.clone();
                                    move |_, window, cx| {
                                        view.update(cx, |view, cx| {
                                            view.details_request.update(cx, |request, cx| {
                                                *request = None;
                                                cx.notify();
                                            });
                                        });
                                        window.close_dialog(cx);
                                    }
                                }),
                        )
                        .child(
                            Button::new("profile-details-save")
                                .debug_selector(|| "profile-details-save".into())
                                .label(tr(lang, "common.save"))
                                .primary()
                                .loading(busy)
                                .disabled(busy)
                                .on_click(move |event, window, cx| {
                                    save_click(event, window, cx);
                                }),
                        ),
                )
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(tr(lang, "common.save"))
                        .cancel_text(tr(lang, "common.cancel"))
                        .show_cancel(true),
                )
                .on_close({
                    let view = view.clone();
                    move |_, _, cx| {
                        view.update(cx, |view, cx| {
                            view.details_request.update(cx, |request, cx| {
                                *request = None;
                                cx.notify();
                            });
                        });
                    }
                })
                .on_ok(move |event, window, cx| save(event, window, cx))
        });
    }

    pub(crate) fn details_response(
        &mut self,
        envelope: &UiResponseEnvelope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if envelope.request_id.is_none() || envelope.request_id != *self.details_request.read(cx) {
            return;
        }
        if let UiResponse::Profile {
            request: AppCommand::UpdateProfileDetails { .. },
            result,
        } = &envelope.response
        {
            self.details_request.update(cx, |request, cx| {
                *request = None;
                cx.notify();
            });
            if result.is_ok() {
                window.close_dialog(cx);
            }
        }
    }
}
