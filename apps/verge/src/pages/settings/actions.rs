use crate::{
    domain::{AppError, ErrorCode, SettingsScope},
    i18n::{self, tr},
    ui::UiAction,
    view::MainView,
};
use gpui_kit::component::{
    WindowExt as _,
    dialog::DialogButtonProps,
    form::{field, v_form},
    v_flex,
};
use gpui_kit::*;

impl MainView {
    pub fn confirm_restore_backup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let lang = self.lang();
        let view = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            alert
                .confirm()
                .title(tr(lang, "dialog.restore_backup.title"))
                .description(tr(lang, "dialog.restore_backup.desc"))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(tr(lang, "dialog.restore_backup.ok"))
                        .ok_variant(gpui_kit::component::button::ButtonVariant::Danger)
                        .cancel_text(tr(lang, "common.cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    view.update(cx, |this, cx| {
                        let passphrase = this.backup_passphrase.read(cx).value().to_string();
                        this.dispatch_confirmed(
                            UiAction::RestoreEncryptedBackup { passphrase },
                            cx,
                        );
                        this.backup_passphrase
                            .update(cx, |input, cx| input.set_value("", window, cx));
                    });
                    true
                })
        });
    }

    /// 卸载特权 Helper 的确认弹窗（移除系统级 LaunchDaemon 与二进制，TUN 将不可用）。
    pub fn confirm_uninstall_helper(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let lang = self.lang();
        let view = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            alert
                .confirm()
                .title(tr(lang, "dialog.uninstall_helper.title"))
                .description(tr(lang, "dialog.uninstall_helper.desc"))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(tr(lang, "settings.system.helper.uninstall"))
                        .ok_variant(gpui_kit::component::button::ButtonVariant::Danger)
                        .cancel_text(tr(lang, "common.cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    view.update(cx, |this, cx| {
                        this.dispatch_confirmed(UiAction::UninstallHelper, cx);
                    });
                    true
                })
        });
    }

    /// 应用更新的确认弹窗（替换当前 .app，重启后生效；失败自动还原现有安装）。
    pub fn confirm_update_application(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let lang = self.lang();
        let view = cx.entity();
        let latest = self
            .state
            .app_update
            .as_ref()
            .map(|status| status.latest_version.clone())
            .unwrap_or_default();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            alert
                .confirm()
                .title(i18n::fmt_update_app_title(lang, &latest))
                .description(tr(lang, "dialog.update_app.desc"))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(tr(lang, "settings.app.update.download"))
                        .cancel_text(tr(lang, "common.cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    view.update(cx, |this, cx| {
                        this.dispatch_confirmed(UiAction::UpdateApplication, cx);
                    });
                    true
                })
        });
    }

    /// 重启应用的确认弹窗（守护进程与 GUI 都退出，新实例接管）。
    pub fn confirm_restart_application(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let lang = self.lang();
        let view = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            alert
                .confirm()
                .title(tr(lang, "dialog.restart.title"))
                .description(tr(lang, "dialog.restart.desc"))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(tr(lang, "dialog.restart.ok"))
                        .cancel_text(tr(lang, "common.cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    view.update(cx, |this, cx| {
                        this.dispatch_confirmed(UiAction::RestartApplication, cx);
                    });
                    true
                })
        });
    }

    /// 恢复默认值的确认弹窗：只重置指定作用域，不动配置、备份和其它设置。
    pub fn confirm_reset_scope(
        &mut self,
        scope: SettingsScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lang = self.lang();
        let scope_label = tr(
            lang,
            match scope {
                SettingsScope::Appearance => "settings.scope.appearance",
                SettingsScope::Network => "settings.scope.network",
                SettingsScope::System => "settings.scope.system",
            },
        );
        let view = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            alert
                .confirm()
                .title(i18n::fmt_reset_scope_title(lang, scope_label))
                .description(tr(lang, "dialog.reset_scope.desc"))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(tr(lang, "dialog.reset_scope.ok"))
                        .cancel_text(tr(lang, "common.cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    view.update(cx, |this, cx| {
                        this.dispatch(UiAction::ResetSettingsScope(scope), cx);
                    });
                    true
                })
        });
    }

    /// 设置导入第一步：发起差异预览，结果回来后由 maybe_open_import_preview_dialog 打开确认弹窗。
    pub fn preview_settings_import(&mut self, cx: &mut Context<Self>) -> Result<(), AppError> {
        let source = self
            .settings_import_path
            .read(cx)
            .value()
            .trim()
            .to_string();
        if source.is_empty() {
            let error = AppError::new(
                ErrorCode::InvalidInput,
                tr(self.lang(), "settings_import.empty_path"),
            );
            self.state.set_error(error.clone());
            cx.notify();
            return Err(error);
        }
        self.state.settings_import_preview = None;
        self.pending_settings_import = Some(source.clone());
        self.dispatch(UiAction::PreviewSettingsImport { source }, cx);
        Ok(())
    }

    /// 预览加载失败时丢弃待处理的确认弹窗（错误由全局提示展示）。
    pub fn fail_import_preview(&mut self) {
        self.pending_settings_import = None;
    }

    /// 预览结果到达后打开确认弹窗：先展示逐字段差异，用户确认后才真正导入。
    pub fn maybe_open_import_preview_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.pending_settings_import.clone() else {
            return;
        };
        let Some(preview) = self.state.settings_import_preview.clone() else {
            return;
        };
        self.pending_settings_import = None;
        let lang = self.lang();
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, window, _| {
            let view = view.clone();
            let source = source.clone();
            let mut changes = v_flex().gap_1();
            if preview.changes.is_empty() {
                changes = changes.child(
                    div()
                        .text_sm()
                        .child(tr(lang, "dialog.import_settings.no_changes")),
                );
            } else {
                for change in &preview.changes {
                    changes = changes.child(div().text_sm().child(i18n::fmt_field_change(
                        lang,
                        &change.field,
                        &change.old,
                        &change.new,
                    )));
                }
            }
            dialog
                .title(tr(lang, "dialog.import_settings.title"))
                .width(rems(30.).to_pixels(window.rem_size()))
                .child(
                    v_form().child(
                        field()
                            .label(tr(lang, "dialog.import_settings.field_diff"))
                            .child(changes),
                    ),
                )
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(tr(lang, "dialog.import_settings.ok"))
                        .cancel_text(tr(lang, "common.cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    view.update(cx, |this, cx| {
                        this.dispatch(
                            UiAction::ImportApplicationSettings {
                                source: source.clone(),
                            },
                            cx,
                        );
                    });
                    true
                })
        });
    }
}
