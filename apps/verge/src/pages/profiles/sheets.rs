use crate::{
    domain::{AppError, ProfileId},
    i18n::{self, tr},
    ui::UiAction,
    view::MainView,
};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::DialogButtonProps,
    h_flex,
    input::Textarea,
    notification::Notification,
    v_flex,
};
use gpui_kit::*;

impl MainView {
    pub fn open_yaml_sheet(&mut self, id: ProfileId, window: &mut Window, cx: &mut Context<Self>) {
        let lang = self.lang();
        self.sheet_state
            .update(cx, |state, _| state.pending_yaml = Some(id.clone()));
        self.yaml_editor.update(cx, |editor, cx| {
            editor.set_value(tr(lang, "sheet.yaml.loading"), window, cx)
        });
        let sheet_state = self.sheet_state.clone();
        let view = cx.entity();
        let editor = self.yaml_editor.clone();
        let mono = cx.theme().mono_font_family.clone();
        let request_id = id.clone();
        window.open_sheet(cx, move |sheet, _, cx| {
            let sheet_state = sheet_state.clone();
            let id = id.clone();
            let editor = editor.clone();
            let mono = mono.clone();
            let loading = sheet_state.read(cx).pending_yaml.as_ref() == Some(&id);
            sheet
                .title(i18n::fmt_titled(lang, "sheet.yaml.title", id.as_str()))
                .size(rems(32.))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .font_family(mono)
                        .text_xs()
                        .child(Textarea::new(&editor).h_full()),
                )
                .footer(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("copy-yaml")
                                .label(tr(lang, "common.copy"))
                                .ghost()
                                .disabled(loading)
                                .on_click({
                                    let editor = editor.clone();
                                    move |_, window, cx| {
                                        let yaml = editor.read(cx).value().to_string();
                                        cx.write_to_clipboard(ClipboardItem::new_string(yaml));
                                        window.push_notification(
                                            Notification::success(tr(lang, "common.copied")),
                                            cx,
                                        );
                                    }
                                }),
                        )
                        .child(
                            Button::new("save-yaml")
                                .label(tr(lang, "sheet.yaml.save"))
                                .primary()
                                .disabled(loading)
                                .on_click({
                                    let view = view.clone();
                                    let id = id.clone();
                                    move |_, window, cx| {
                                        let view = view.clone();
                                        let id = id.clone();
                                        window.open_alert_dialog(cx, move |alert, _, _| {
                                            let view = view.clone();
                                            let id = id.clone();
                                            alert
                                                .confirm()
                                                .title(i18n::fmt_save_yaml_title(lang, id.as_str()))
                                                .description(tr(lang, "sheet.yaml.confirm_desc"))
                                                .button_props(
                                                    DialogButtonProps::default()
                                                        .ok_text(tr(lang, "common.save"))
                                                        .cancel_text(tr(lang, "common.cancel"))
                                                        .show_cancel(true),
                                                )
                                                .on_ok(move |_, window, cx| {
                                                    view.update(cx, |this, cx| {
                                                        let yaml = this
                                                            .yaml_editor
                                                            .read(cx)
                                                            .value()
                                                            .to_string();
                                                        this.dispatch(
                                                            UiAction::UpdateProfileYaml {
                                                                id: id.clone(),
                                                                yaml,
                                                            },
                                                            cx,
                                                        );
                                                    });
                                                    window.close_sheet(cx);
                                                    true
                                                })
                                        });
                                    }
                                }),
                        ),
                )
        });
        self.dispatch(UiAction::LoadProfileYaml(request_id), cx);
    }

    /// YAML 加载成功后填入已经打开的 Sheet。
    pub fn maybe_open_yaml_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let want = self.sheet_state.read(cx).pending_yaml.clone();
        let Some(want) = want else {
            return;
        };
        let Some((id, yaml)) = self.state.profile_yaml.clone() else {
            return;
        };
        if id != want {
            return;
        }
        self.sheet_state
            .update(cx, |state, _| state.pending_yaml = None);
        self.yaml_editor
            .update(cx, |editor, cx| editor.set_value(yaml.clone(), window, cx));
    }

    /// YAML 加载失败时保留 Sheet，并在编辑器内直接显示错误。
    pub fn fail_yaml_sheet(
        &mut self,
        error: &AppError,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sheet_state
            .update(cx, |state, _| state.pending_yaml = None);
        self.yaml_editor.update(cx, |editor, cx| {
            editor.set_value(
                i18n::fmt_load_failed(self.lang(), "sheet.yaml.load_failed", &error.message),
                window,
                cx,
            )
        });
    }

    /// 点击后立即打开 Merge 配置 Sheet；加载完成后由响应轮询填入编辑器。
    pub fn open_merge_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let lang = self.lang();
        self.sheet_state
            .update(cx, |state, _| state.pending_merge = true);
        self.merge_editor.update(cx, |editor, cx| {
            editor.set_value(tr(lang, "sheet.merge.loading"), window, cx)
        });
        let view = cx.entity();
        let sheet_state = self.sheet_state.clone();
        let editor = self.merge_editor.clone();
        let mono = cx.theme().mono_font_family.clone();
        window.open_sheet(cx, move |sheet, _, cx| {
            let view = view.clone();
            let sheet_state = sheet_state.clone();
            let editor = editor.clone();
            let mono = mono.clone();
            let loading = sheet_state.read(cx).pending_merge;
            sheet
                .title(tr(lang, "sheet.merge.title"))
                .size(rems(32.))
                .child(
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr(lang, "sheet.merge.desc")),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .font_family(mono)
                                .text_xs()
                                .child(Textarea::new(&editor).h_full()),
                        ),
                )
                .footer(
                    h_flex().gap_2().justify_end().child(
                        Button::new("save-merge")
                            .label(tr(lang, "sheet.merge.save"))
                            .primary()
                            .disabled(loading)
                            .on_click({
                                let view = view.clone();
                                move |_, window, cx| {
                                    let view = view.clone();
                                    window.open_alert_dialog(cx, move |alert, _, _| {
                                        let view = view.clone();
                                        alert
                                            .confirm()
                                            .title(tr(lang, "sheet.merge.confirm_title"))
                                            .description(tr(lang, "sheet.merge.confirm_desc"))
                                            .button_props(
                                                DialogButtonProps::default()
                                                    .ok_text(tr(lang, "common.save"))
                                                    .cancel_text(tr(lang, "common.cancel"))
                                                    .show_cancel(true),
                                            )
                                            .on_ok(move |_, _, cx| {
                                                view.update(cx, |this, cx| {
                                                    let yaml = this
                                                        .merge_editor
                                                        .read(cx)
                                                        .value()
                                                        .to_string();
                                                    this.dispatch(
                                                        UiAction::SaveMergeConfig { yaml },
                                                        cx,
                                                    );
                                                });
                                                true
                                            })
                                    });
                                }
                            }),
                    ),
                )
        });
        self.dispatch(UiAction::LoadMergeConfig, cx);
    }

    /// Merge 配置加载成功后填入已经打开的 Sheet。
    pub fn maybe_open_merge_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.sheet_state.read(cx).pending_merge {
            return;
        }
        let Some(yaml) = self.state.merge_yaml.clone() else {
            return;
        };
        self.sheet_state
            .update(cx, |state, _| state.pending_merge = false);
        self.merge_editor
            .update(cx, |editor, cx| editor.set_value(yaml.clone(), window, cx));
    }

    /// Merge 配置加载失败时保留 Sheet，并在编辑器内直接显示错误。
    pub fn fail_merge_sheet(
        &mut self,
        error: &AppError,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sheet_state
            .update(cx, |state, _| state.pending_merge = false);
        self.merge_editor.update(cx, |editor, cx| {
            editor.set_value(
                i18n::fmt_load_failed(self.lang(), "sheet.merge.load_failed", &error.message),
                window,
                cx,
            )
        });
    }

    /// 点击后立即打开合并结果 Sheet（只读）；加载完成后由响应轮询填入内容。
    pub fn open_merged_sheet(
        &mut self,
        id: ProfileId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lang = self.lang();
        self.sheet_state
            .update(cx, |state, _| state.pending_merged = Some(id.clone()));
        self.merged_editor.update(cx, |editor, cx| {
            editor.set_value(tr(lang, "sheet.merged.loading"), window, cx)
        });
        let sheet_state = self.sheet_state.clone();
        let editor = self.merged_editor.clone();
        let mono = cx.theme().mono_font_family.clone();
        let request_id = id.clone();
        window.open_sheet(cx, move |sheet, _, cx| {
            let editor = editor.clone();
            let mono = mono.clone();
            let sheet_state = sheet_state.clone();
            let loading = sheet_state.read(cx).pending_merged.is_some();
            sheet
                .title(i18n::fmt_titled(lang, "sheet.merged.title", id.as_str()))
                .size(rems(32.))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .font_family(mono)
                        .text_xs()
                        .child(Textarea::new(&editor).h_full()),
                )
                .footer(
                    h_flex().gap_2().justify_end().child(
                        Button::new("copy-merged")
                            .label(tr(lang, "common.copy"))
                            .ghost()
                            .disabled(loading)
                            .on_click({
                                let editor = editor.clone();
                                move |_, window, cx| {
                                    let yaml = editor.read(cx).value().to_string();
                                    cx.write_to_clipboard(ClipboardItem::new_string(yaml));
                                    window.push_notification(
                                        Notification::success(tr(lang, "common.copied")),
                                        cx,
                                    );
                                }
                            }),
                    ),
                )
        });
        self.dispatch(UiAction::LoadMergedYaml(request_id), cx);
    }

    /// 合并结果加载成功后填入已经打开的 Sheet。
    pub fn maybe_open_merged_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let want = self.sheet_state.read(cx).pending_merged.clone();
        let Some(want) = want else {
            return;
        };
        let Some((id, yaml)) = self.state.merged_yaml.clone() else {
            return;
        };
        if id != want {
            return;
        }
        self.sheet_state
            .update(cx, |state, _| state.pending_merged = None);
        self.merged_editor
            .update(cx, |editor, cx| editor.set_value(yaml.clone(), window, cx));
    }

    /// 合并结果生成失败时保留 Sheet，并在编辑器内直接显示错误。
    pub fn fail_merged_sheet(
        &mut self,
        error: &AppError,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sheet_state
            .update(cx, |state, _| state.pending_merged = None);
        self.merged_editor.update(cx, |editor, cx| {
            editor.set_value(
                i18n::fmt_load_failed(self.lang(), "sheet.merged.load_failed", &error.message),
                window,
                cx,
            )
        });
    }
}
