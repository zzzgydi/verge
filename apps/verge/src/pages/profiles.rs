mod actions;
mod sheets;
use super::components::{page_heading, panel};
use crate::{
    domain::{Profile, ProfileSource, UpdatePolicy},
    format,
    i18n::{Lang, tr},
    ui::UiAction,
    view::MainView,
};
use gpui::{prelude::FluentBuilder as _, *};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    v_flex,
};

fn policy_summary(lang: Lang, profile: &Profile) -> String {
    match &profile.update_policy {
        UpdatePolicy::Manual => tr(lang, "profiles.policy.manual").to_owned(),
        UpdatePolicy::Interval { seconds } => format!(
            "{} · {} {} · {} {}{}",
            format::interval(lang, *seconds),
            tr(lang, "profiles.policy.next"),
            profile.next_update_at.map_or_else(
                || tr(lang, "profiles.policy.unscheduled").into(),
                |at| at.to_string()
            ),
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
    let mut list = v_flex().gap_4();
    if view.state.profiles.is_empty() {
        list = list.child(if refreshing {
            super::skeleton_rows(2).into_any_element()
        } else {
            super::EmptyState::new(
                IconName::File,
                tr(lang, "profiles.empty.title"),
                tr(lang, "profiles.empty.desc"),
            )
            .action(
                Button::new("empty-import")
                    .label(tr(lang, "profiles.import"))
                    .small()
                    .primary()
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_import_dialog(window, cx)),
                    ),
            )
            .into_any_element()
        });
    }
    for profile in &view.state.profiles {
        let id = profile.id.clone();
        let selected = view.state.selected_profile.as_ref() == Some(&id);
        let remote = matches!(profile.source, ProfileSource::Remote { .. });
        let source = tr(
            lang,
            if remote {
                "profiles.source.remote"
            } else {
                "profiles.source.local"
            },
        );
        let actions = h_flex()
            .gap_2()
            .child(
                Button::new(format!("load-profile-{}", id.as_str()))
                    .label(tr(lang, "profiles.view_yaml"))
                    .small()
                    .ghost()
                    .loading(view.is_pending(&["profile_yaml"]))
                    .on_click(cx.listener({
                        let id = id.clone();
                        move |this, _, window, cx| this.open_yaml_sheet(id.clone(), window, cx)
                    })),
            )
            .when(remote, |row| {
                row.child(
                    Button::new(format!("update-profile-{}", id.as_str()))
                        .label(tr(lang, "common.update_now"))
                        .small()
                        .ghost()
                        .on_click(cx.listener({
                            let id = id.clone();
                            move |this, _, _, cx| {
                                this.dispatch(UiAction::UpdateRemoteProfile(id.clone()), cx)
                            }
                        })),
                )
            })
            .child(
                Button::new(format!("select-profile-{}", id.as_str()))
                    .label(tr(
                        lang,
                        if selected {
                            "profiles.selected"
                        } else {
                            "profiles.select"
                        },
                    ))
                    .small()
                    .primary()
                    .disabled(selected)
                    .loading(view.is_pending(&["profile_write"]))
                    .on_click(cx.listener({
                        let id = id.clone();
                        move |this, _, _, cx| this.dispatch(UiAction::SelectProfile(id.clone()), cx)
                    })),
            )
            .child(
                Button::new(format!("profile-more-{}", id.as_str()))
                    .icon(IconName::Ellipsis)
                    .small()
                    .ghost()
                    .tooltip(tr(lang, "common.more"))
                    .dropdown_menu({
                        let entity = cx.entity().downgrade();
                        let id = id.clone();
                        let name = profile.name.clone();
                        move |menu, _, _| {
                            let merged_entity = entity.clone();
                            let merged_id = id.clone();
                            let policy_entity = entity.clone();
                            let policy_id = id.clone();
                            let policy_name = name.clone();
                            let delete_entity = entity.clone();
                            let delete_id = id.clone();
                            let delete_name = name.clone();
                            menu.item(PopupMenuItem::new(tr(lang, "profiles.merged")).on_click(
                                move |_, window, cx| {
                                    let _ = merged_entity.update(cx, |this, cx| {
                                        this.open_merged_sheet(merged_id.clone(), window, cx)
                                    });
                                },
                            ))
                            .when(remote, |menu| {
                                menu.item(
                                    PopupMenuItem::new(tr(lang, "profiles.set_interval")).on_click(
                                        move |_, window, cx| {
                                            let _ = policy_entity.update(cx, |this, cx| {
                                                this.open_interval_dialog(
                                                    policy_id.clone(),
                                                    policy_name.clone(),
                                                    window,
                                                    cx,
                                                )
                                            });
                                        },
                                    ),
                                )
                            })
                            .separator()
                            .item(
                                PopupMenuItem::new(tr(lang, "profiles.delete_menu")).on_click(
                                    move |_, window, cx| {
                                        let _ = delete_entity.update(cx, |this, cx| {
                                            this.confirm_delete_profile(
                                                delete_id.clone(),
                                                delete_name.clone(),
                                                window,
                                                cx,
                                            )
                                        });
                                    },
                                ),
                            )
                        }
                    }),
            );
        list = list.child(
            panel(cx)
                .id(SharedString::from(format!("profile-{}", id.as_str())))
                .p_0()
                .gap_0()
                .overflow_hidden()
                .when(selected, |card| {
                    card.border_color(cx.theme().list_active_border)
                        .bg(cx.theme().list_active.opacity(0.35))
                })
                .child(
                    h_flex()
                        .px_5()
                        .py_5()
                        .gap_4()
                        .child(
                            div()
                                .size_10()
                                .flex_shrink_0()
                                .rounded_lg()
                                .bg(cx.theme().accent)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    Icon::new(if remote {
                                        IconName::Globe
                                    } else {
                                        IconName::File
                                    })
                                    .size_5(),
                                ),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                .child(
                                    div()
                                        .text_lg()
                                        .font_medium()
                                        .truncate()
                                        .child(profile.name.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(source),
                                ),
                        )
                        .when(selected, |row| {
                            row.child(
                                h_flex()
                                    .gap_2()
                                    .text_xs()
                                    .child(div().size(px(6.)).rounded_full().bg(cx.theme().success))
                                    .child(tr(lang, "profiles.current")),
                            )
                        }),
                )
                .child(
                    h_flex()
                        .px_5()
                        .py_3()
                        .gap_3()
                        .flex_wrap()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(160.))
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(policy_summary(lang, profile)),
                        )
                        .child(actions),
                ),
        );
    }
    v_flex()
        .gap_6()
        .child(
            h_flex()
                .justify_between()
                .child(page_heading(
                    tr(lang, "profiles.title"),
                    tr(lang, "profiles.subtitle"),
                    cx,
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("refresh-profiles")
                                .icon(IconName::Redo)
                                .tooltip(tr(lang, "common.refresh"))
                                .small()
                                .ghost()
                                .loading(refreshing)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.dispatch(UiAction::RefreshProfiles, cx)
                                })),
                        )
                        .child(
                            Button::new("open-merge-sheet")
                                .label(tr(lang, "profiles.merge_config"))
                                .small()
                                .outline()
                                .loading(view.is_pending(&["merge_config"]))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_merge_sheet(window, cx)
                                })),
                        )
                        .child(
                            Button::new("open-import-dialog")
                                .label(tr(lang, "profiles.import"))
                                .icon(IconName::Plus)
                                .small()
                                .primary()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_import_dialog(window, cx)
                                })),
                        ),
                ),
        )
        .child(list)
        .into_any_element()
}
