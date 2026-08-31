use gpui::{prelude::FluentBuilder as _, *};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    group_box::GroupBox,
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    tag::Tag,
    v_flex,
};
use verge_domain::{Profile, ProfileSource, UpdatePolicy};
use verge_ui::UiAction;

use crate::{
    format,
    i18n::{Lang, tr},
    view::MainView,
};

use super::page_title;

fn policy_summary(lang: Lang, profile: &Profile) -> String {
    match &profile.update_policy {
        UpdatePolicy::Manual => tr(lang, "profiles.policy.manual").to_owned(),
        UpdatePolicy::Interval { seconds } => format!(
            "{} · {} {} · {} {}{}",
            format::interval(lang, *seconds),
            tr(lang, "profiles.policy.next"),
            profile
                .next_update_at
                .map_or_else(|| tr(lang, "profiles.policy.unscheduled").into(), |at| at.to_string()),
            tr(lang, "profiles.policy.failures"),
            profile.consecutive_failures,
            profile
                .last_error
                .as_ref()
                .map_or_else(String::new, |error| format!(" · {error}"))
        ),
    }
}

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let lang = view.lang();
    let refreshing = view.is_pending(&["profiles"]);
    let mut list = v_flex().gap_3();
    if view.state.profiles.is_empty() {
        if refreshing {
            list = list.child(super::skeleton_rows(2));
        } else {
            list = list.child(
                super::EmptyState::new(
                    gpui_component::IconName::File,
                    tr(lang, "profiles.empty.title"),
                    tr(lang, "profiles.empty.desc"),
                )
                .action(
                    Button::new("empty-import")
                        .label(tr(lang, "profiles.import"))
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
            ProfileSource::Local => tr(lang, "profiles.source.local"),
            ProfileSource::Remote { .. } => tr(lang, "profiles.source.remote"),
        };
        // 操作分级：启用是主操作（primary），查看/合并/更新是次要（ghost），
        // 删除收进"更多…"下拉（危险操作隔离，确认弹窗内才是 danger 按钮）。
        let actions = h_flex()
            .gap_1()
            .child(
                Button::new(format!("select-profile-{}", id.as_str()))
                    .label(if selected {
                        tr(lang, "profiles.selected")
                    } else {
                        tr(lang, "profiles.select")
                    })
                    .small()
                    .primary()
                    .disabled(selected)
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
                    .label(tr(lang, "profiles.view_yaml"))
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
                    .label(tr(lang, "profiles.merged"))
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
                            .label(tr(lang, "common.update_now"))
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
                            .label(tr(lang, "profiles.set_interval"))
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
                Button::new(format!("profile-more-{}", id.as_str()))
                    .label(tr(lang, "common.more"))
                    .small()
                    .ghost()
                    .dropdown_menu({
                        let view_entity = cx.entity();
                        let id = id.clone();
                        let name = name.clone();
                        move |menu, _, _| {
                            menu.item(
                                PopupMenuItem::new(tr(lang, "profiles.delete_menu")).on_click({
                                    let view = view_entity.clone();
                                    let id = id.clone();
                                    let name = name.clone();
                                    move |_, window, cx| {
                                        view.update(cx, |this, cx| {
                                            this.confirm_delete_profile(id.clone(), name.clone(), window, cx);
                                        });
                                    }
                                }),
                            )
                        }
                    }),
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
                                    this.child(
                                        Tag::success()
                                            .small()
                                            .child(tr(lang, "profiles.current")),
                                    )
                                })
                                .child(
                                    super::selectable_text(
                                        format!("profile-meta-{}", profile.id.as_str()),
                                        format!("{source} · {}", policy_summary(lang, profile)),
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
            h_flex()
                .justify_between()
                .child(page_title(tr(lang, "profiles.title")))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("refresh-profiles")
                                .label(tr(lang, "common.refresh"))
                                .small()
                                .ghost()
                                .loading(refreshing)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.dispatch(UiAction::RefreshProfiles, cx);
                                })),
                        )
                        .child(
                            Button::new("open-merge-sheet")
                                .label(tr(lang, "profiles.merge_config"))
                                .small()
                                .ghost()
                                .loading(view.is_pending(&["merge_config"]))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_merge_sheet(window, cx);
                                })),
                        )
                        .child(
                            Button::new("open-import-dialog")
                                .label(tr(lang, "profiles.import"))
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
