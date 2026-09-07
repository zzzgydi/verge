use crate::{
    domain::{AppError, ErrorCode, ProfileId, ProfileSource, UpdatePolicy},
    i18n::{self, tr},
    ui::UiAction,
    view::MainView,
};
use gpui::*;
use gpui_component::{
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::DialogButtonProps,
    form::{field, v_form},
    h_flex,
    input::{Input, Textarea},
    notification::Notification,
};

impl MainView {
    pub fn import_profile(&mut self, cx: &mut Context<Self>) -> Result<(), AppError> {
        let id = self.profile_id.read(cx).value().to_string();
        let name = self.profile_name.read(cx).value().to_string();
        let yaml = self.profile_yaml.read(cx).value().to_string();
        match ProfileId::parse(id) {
            Ok(id) => {
                self.dispatch(
                    UiAction::ImportProfile {
                        id,
                        name,
                        yaml,
                        source: ProfileSource::Local,
                        update_policy: UpdatePolicy::Manual,
                    },
                    cx,
                );
                Ok(())
            }
            Err(error) => {
                self.state.set_error(error.clone());
                cx.notify();
                Err(error)
            }
        }
    }

    pub fn import_remote_profile(&mut self, cx: &mut Context<Self>) -> Result<(), AppError> {
        let id = self.profile_id.read(cx).value().to_string();
        let name = self.profile_name.read(cx).value().to_string();
        let url = self.profile_url.read(cx).value().to_string();
        let interval = self.profile_interval.read(cx).value().parse::<u64>();
        let user_agent = self.profile_user_agent.read(cx).value().trim().to_string();
        let user_agent = (!user_agent.is_empty()).then_some(user_agent);
        match (ProfileId::parse(id), interval) {
            (Ok(id), Ok(seconds)) if seconds > 0 => {
                self.dispatch(
                    UiAction::ImportRemoteProfile {
                        id,
                        name,
                        url,
                        update_policy: UpdatePolicy::Interval { seconds },
                        user_agent,
                    },
                    cx,
                );
                Ok(())
            }
            (Err(error), _) => {
                self.state.set_error(error.clone());
                cx.notify();
                Err(error)
            }
            _ => {
                let error = AppError::new(
                    ErrorCode::InvalidInput,
                    "update interval must be a positive integer",
                );
                self.state.set_error(error.clone());
                cx.notify();
                Err(error)
            }
        }
    }

    pub fn set_update_interval(
        &mut self,
        id: ProfileId,
        cx: &mut Context<Self>,
    ) -> Result<(), AppError> {
        match self.profile_interval.read(cx).value().parse::<u64>() {
            Ok(seconds) if seconds > 0 => {
                self.dispatch(
                    UiAction::SetProfileUpdatePolicy {
                        id,
                        update_policy: UpdatePolicy::Interval { seconds },
                    },
                    cx,
                );
                Ok(())
            }
            _ => {
                let error = AppError::new(
                    ErrorCode::InvalidInput,
                    "update interval must be a positive integer",
                );
                self.state.set_error(error.clone());
                cx.notify();
                Err(error)
            }
        }
    }

    /// 导入配置对话框：表单字段持有 MainView 上的输入状态，确认后 dispatch。
    pub fn open_import_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let lang = self.lang();
        let view = cx.entity();
        let id_input = self.profile_id.clone();
        let name_input = self.profile_name.clone();
        let url_input = self.profile_url.clone();
        let interval_input = self.profile_interval.clone();
        let ua_input = self.profile_user_agent.clone();
        let yaml_input = self.profile_yaml.clone();
        window.open_dialog(cx, move |dialog, window, _| {
            let view_local = view.clone();
            let view_remote = view.clone();
            dialog
                .title(tr(lang, "dialog.import.title"))
                .width(rems(35.).to_pixels(window.rem_size()))
                .child(
                    v_form()
                        .child(
                            field()
                                .label(tr(lang, "dialog.import.id"))
                                .required(true)
                                .child(Input::new(&id_input)),
                        )
                        .child(
                            field()
                                .label(tr(lang, "dialog.import.name"))
                                .required(true)
                                .child(Input::new(&name_input)),
                        )
                        .child(
                            field()
                                .label(tr(lang, "dialog.import.url"))
                                .description(tr(lang, "dialog.import.url.desc"))
                                .child(Input::new(&url_input)),
                        )
                        .child(
                            field()
                                .label(tr(lang, "dialog.import.interval"))
                                .child(Input::new(&interval_input)),
                        )
                        .child(
                            field()
                                .label("User-Agent")
                                .description(tr(lang, "dialog.import.user_agent.desc"))
                                .child(Input::new(&ua_input)),
                        )
                        .child(
                            field()
                                .label(tr(lang, "dialog.import.yaml"))
                                .child(Textarea::new(&yaml_input).h_32()),
                        ),
                )
                .footer(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("cancel")
                                .label(tr(lang, "common.cancel"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("import-local")
                                .label(tr(lang, "dialog.import.local"))
                                .outline()
                                .on_click(move |_, window, cx| {
                                    match view_local.update(cx, |this, cx| this.import_profile(cx))
                                    {
                                        Ok(()) => window.close_dialog(cx),
                                        Err(error) => window.push_notification(
                                            Notification::error(error.message).autohide(false),
                                            cx,
                                        ),
                                    }
                                }),
                        )
                        .child(
                            Button::new("import-remote")
                                .label(tr(lang, "dialog.import.remote"))
                                .primary()
                                .on_click(move |_, window, cx| {
                                    match view_remote
                                        .update(cx, |this, cx| this.import_remote_profile(cx))
                                    {
                                        Ok(()) => window.close_dialog(cx),
                                        Err(error) => window.push_notification(
                                            Notification::error(error.message).autohide(false),
                                            cx,
                                        ),
                                    }
                                }),
                        ),
                )
        });
    }

    /// “设置间隔”对话框：复用更新间隔输入框，保存到指定配置。
    pub fn open_interval_dialog(
        &mut self,
        id: ProfileId,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lang = self.lang();
        let view = cx.entity();
        let interval_input = self.profile_interval.clone();
        window.open_dialog(cx, move |dialog, window, _| {
            let view = view.clone();
            let id = id.clone();
            dialog
                .title(i18n::fmt_titled(lang, "dialog.interval.title", &name))
                .width(rems(26.).to_pixels(window.rem_size()))
                .child(
                    v_form().child(
                        field()
                            .label(tr(lang, "dialog.import.interval"))
                            .description(tr(lang, "dialog.interval.desc"))
                            .child(Input::new(&interval_input)),
                    ),
                )
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(tr(lang, "common.save"))
                        .cancel_text(tr(lang, "common.cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    match view.update(cx, |this, cx| this.set_update_interval(id.clone(), cx)) {
                        Ok(()) => true,
                        Err(error) => {
                            window.push_notification(
                                Notification::error(error.message).autohide(false),
                                cx,
                            );
                            false
                        }
                    }
                })
        });
    }

    /// 删除配置的确认弹窗。
    pub fn confirm_delete_profile(
        &mut self,
        id: ProfileId,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lang = self.lang();
        let view = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            let id = id.clone();
            alert
                .confirm()
                .title(i18n::fmt_delete_profile_title(lang, &name))
                .description(tr(lang, "dialog.delete_profile.desc"))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(tr(lang, "common.delete"))
                        .ok_variant(gpui_component::button::ButtonVariant::Danger)
                        .cancel_text(tr(lang, "common.cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    view.update(cx, |this, cx| {
                        this.dispatch_confirmed(UiAction::DeleteProfile(id.clone()), cx);
                    });
                    true
                })
        });
    }
}
