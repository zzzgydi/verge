use gpui::{prelude::FluentBuilder as _, *};
use gpui_component::{
    ActiveTheme as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    group_box::GroupBox,
    h_flex,
    tag::Tag,
    v_flex,
};
use verge_domain::{Profile, ProfileSource, UpdatePolicy};
use verge_ui::UiAction;

use crate::{format, view::MainView};

use super::page_title;

fn policy_summary(profile: &Profile) -> String {
    match &profile.update_policy {
        UpdatePolicy::Manual => "手动更新".to_owned(),
        UpdatePolicy::Interval { seconds } => format!(
            "{} · 下次更新 {} · 连续失败 {}{}",
            format::interval(*seconds),
            profile
                .next_update_at
                .map_or_else(|| "未安排".into(), |at| at.to_string()),
            profile.consecutive_failures,
            profile
                .last_error
                .as_ref()
                .map_or_else(String::new, |error| format!(" · {error}"))
        ),
    }
}

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let refreshing = view.is_pending(&["profiles"]);
    let mut list = v_flex().gap_3();
    if view.state.profiles.is_empty() {
        if refreshing {
            list = list.child(super::skeleton_rows(2));
        } else {
            list = list.child(
                super::EmptyState::new(
                    gpui_component::IconName::File,
                    "暂无配置",
                    "导入本地 YAML 或远程订阅后开始使用。",
                )
                .action(
                    Button::new("empty-import")
                        .label("导入配置…")
                        .small()
                        .primary()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_import_dialog(window, cx);
                        })),
                ),
            );
        }
    }
    for profile in &view.state.profiles {
        let id = profile.id.clone();
        let name = profile.name.clone();
        let selected = view.state.selected_profile.as_ref() == Some(&id);
        let source = match &profile.source {
            ProfileSource::Local => "本地",
            ProfileSource::Remote { .. } => "远程",
        };
        let actions = h_flex()
            .gap_1()
            .child(
                Button::new(format!("select-profile-{}", id.as_str()))
                    .label("启用")
                    .small()
                    .ghost()
                    .selected(selected)
                    .loading(view.is_pending(&["profile_write"]))
                    .on_click(cx.listener({
                        let id = id.clone();
                        move |this, _, _, cx| {
                            this.dispatch(UiAction::SelectProfile(id.clone()), cx);
                        }
                    })),
            )
            .child(
                Button::new(format!("load-profile-{}", id.as_str()))
                    .label("查看 YAML…")
                    .small()
                    .ghost()
                    .loading(view.is_pending(&["profile_yaml"]))
                    .on_click(cx.listener({
                        let id = id.clone();
                        move |this, _, window, cx| {
                            this.open_yaml_sheet(id.clone(), window, cx);
                        }
                    })),
            )
            .child(
                Button::new(format!("merged-profile-{}", id.as_str()))
                    .label("合并结果…")
                    .small()
                    .ghost()
                    .loading(view.is_pending(&["merged_yaml"]))
                    .on_click(cx.listener({
                        let id = id.clone();
                        move |this, _, window, cx| {
                            this.open_merged_sheet(id.clone(), window, cx);
                        }
                    })),
            )
            .when(
                matches!(profile.source, ProfileSource::Remote { .. }),
                |row| {
                    row.child(
                        Button::new(format!("update-profile-{}", id.as_str()))
                            .label("立即更新")
                            .small()
                            .ghost()
                            .on_click(cx.listener({
                                let id = id.clone();
                                move |this, _, _, cx| {
                                    this.dispatch(UiAction::UpdateRemoteProfile(id.clone()), cx);
                                }
                            })),
                    )
                    .child(
                        Button::new(format!("policy-profile-{}", id.as_str()))
                            .label("设置间隔…")
                            .small()
                            .ghost()
                            .on_click(cx.listener({
                                let id = id.clone();
                                let name = name.clone();
                                move |this, _, window, cx| {
                                    this.open_interval_dialog(id.clone(), name.clone(), window, cx);
                                }
                            })),
                    )
                },
            )
            .child(
                Button::new(format!("delete-profile-{}", id.as_str()))
                    .label("删除")
                    .small()
                    .ghost()
                    .danger()
                    .on_click(cx.listener({
                        let id = id.clone();
                        let name = name.clone();
                        move |this, _, window, cx| {
                            this.confirm_delete_profile(id.clone(), name.clone(), window, cx);
                        }
                    })),
            );
        list = list.child(
            GroupBox::new()
                .id(SharedString::from(format!(
                    "profile-{}",
                    profile.id.as_str()
                )))
                .title(profile.name.clone())
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            h_flex()
                                .gap_2()
                                .when(selected, |this| {
                                    this.child(Tag::success().small().child("当前"))
                                })
                                .child(
                                    super::selectable_text(
                                        format!("profile-meta-{}", profile.id.as_str()),
                                        format!("{source} · {}", policy_summary(profile)),
                                    )
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground),
                                ),
                        )
                        .child(actions),
                ),
        );
    }

    v_flex()
        .gap_4()
        .child(
            h_flex().justify_between().child(page_title("配置")).child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("refresh-profiles")
                            .label("刷新")
                            .small()
                            .ghost()
                            .loading(refreshing)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.dispatch(UiAction::RefreshProfiles, cx);
                            })),
                    )
                    .child(
                        Button::new("open-merge-sheet")
                            .label("Merge 配置…")
                            .small()
                            .ghost()
                            .loading(view.is_pending(&["merge_config"]))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_merge_sheet(window, cx);
                            })),
                    )
                    .child(
                        Button::new("open-import-dialog")
                            .label("导入配置…")
                            .small()
                            .primary()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_import_dialog(window, cx);
                            })),
                    ),
            ),
        )
        .child(list)
        .into_any_element()
}
