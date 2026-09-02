use std::rc::Rc;

use crate::domain::ProviderKind;
use crate::ui::UiAction;
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    group_box::GroupBox,
    h_flex, v_flex, v_virtual_list,
};

use crate::{
    i18n::{self, tr},
    view::MainView,
};

use super::{muted, page_title};

/// 规则行高。虚拟列表要求声明尺寸与实际渲染高度一致。
const RULE_ROW_HEIGHT: f32 = 28.;

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let lang = view.lang();
    let mono = cx.theme().mono_font_family.clone();
    let refreshing = view.is_pending(&["rules", "providers"]);

    let mut providers = v_flex().gap_1();
    if view.state.providers.is_empty() && !refreshing {
        providers = providers.child(muted(tr(lang, "rules.providers.empty"), cx));
    }
    for provider in &view.state.providers {
        let name = provider.name.clone();
        let kind = provider.kind;
        let kind_label = match kind {
            ProviderKind::Proxy => tr(lang, "rules.provider_kind.proxy"),
            ProviderKind::Rule => tr(lang, "rules.provider_kind.rule"),
        };
        providers = providers.child(
            h_flex()
                .w_full()
                .gap_2()
                .justify_between()
                .child(
                    super::selectable_text(
                        format!("provider-{kind:?}-{name}"),
                        i18n::fmt_provider_summary(
                            lang,
                            kind_label,
                            &provider.name,
                            &provider.vehicle,
                            provider.item_count,
                            &provider.updated_at,
                        ),
                    )
                    .text_sm()
                    .text_color(cx.theme().muted_foreground),
                )
                .child(
                    Button::new(format!("update-provider-{kind:?}-{name}"))
                        .label(tr(lang, "rules.update"))
                        .xsmall()
                        .ghost()
                        .loading(view.is_pending(&["provider_write"]))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.dispatch(
                                UiAction::UpdateProvider {
                                    kind,
                                    name: name.clone(),
                                },
                                cx,
                            );
                        })),
                ),
        );
    }

    let rules = if view.state.rules.is_empty() {
        if refreshing {
            super::skeleton_rows(5).into_any_element()
        } else {
            super::EmptyState::new(
                gpui_component::IconName::BookOpen,
                tr(lang, "rules.empty.title"),
                tr(lang, "rules.empty.desc"),
            )
            .action(
                Button::new("empty-refresh-rules")
                    .label(tr(lang, "common.refresh"))
                    .small()
                    .outline()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.dispatch(UiAction::RefreshRules, cx);
                    })),
            )
            .into_any_element()
        }
    } else {
        let item_sizes = Rc::new(vec![
            size(px(0.), px(RULE_ROW_HEIGHT));
            view.state.rules.len()
        ]);
        let view_entity = cx.entity();
        div()
            .flex_1()
            .min_h_0()
            .child(v_virtual_list(
                view_entity,
                "rule-list",
                item_sizes,
                move |this, range, _, _cx| {
                    range
                        .filter_map(|ix| this.state.rules.get(ix))
                        .map(|rule| {
                            div()
                                .h(px(RULE_ROW_HEIGHT))
                                .w_full()
                                .flex()
                                .items_center()
                                .overflow_hidden()
                                .child(
                                    super::selectable_text(
                                        format!("rule-{}-{}", rule.kind, rule.payload),
                                        format!(
                                            "{} · {} · {} → {}",
                                            rule.kind, rule.payload, rule.size, rule.proxy
                                        ),
                                    )
                                    .font_family(mono.clone())
                                    .text_sm()
                                    .max_lines(1),
                                )
                                .into_any_element()
                        })
                        .collect()
                },
            ))
            .into_any_element()
    };

    v_flex()
        .size_full()
        .gap_4()
        .child(
            h_flex()
                .justify_between()
                .child(page_title(tr(lang, "rules.title")))
                .child(
                    Button::new("refresh-rules")
                        .label(tr(lang, "common.refresh"))
                        .small()
                        .ghost()
                        .loading(refreshing)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.dispatch(UiAction::RefreshRules, cx);
                        })),
                ),
        )
        .child(
            GroupBox::new()
                .id("rule-providers")
                .title("Providers")
                .child(providers),
        )
        .child(
            v_flex()
                .id("rules-section")
                .flex_1()
                .min_h_0()
                .gap_4()
                .child(muted(tr(lang, "rules.section"), cx))
                .child(rules),
        )
        .into_any_element()
}
