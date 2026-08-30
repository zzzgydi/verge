use std::rc::Rc;

use gpui::*;
use gpui_component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    group_box::GroupBox,
    h_flex, v_flex, v_virtual_list,
};
use verge_domain::ProviderKind;
use verge_ui::UiAction;

use crate::view::MainView;

use super::{muted, page_title};

/// 规则行高。虚拟列表要求声明尺寸与实际渲染高度一致。
const RULE_ROW_HEIGHT: f32 = 28.;

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let mono = cx.theme().mono_font_family.clone();
    let refreshing = view.is_pending(&["rules", "providers"]);

    let mut providers = v_flex().gap_1();
    if view.state.providers.is_empty() && !refreshing {
        providers = providers.child(muted("暂无 Provider。", cx));
    }
    for provider in &view.state.providers {
        let name = provider.name.clone();
        let kind = provider.kind;
        let kind_label = match kind {
            ProviderKind::Proxy => "代理",
            ProviderKind::Rule => "规则",
        };
        providers = providers.child(
            h_flex()
                .w_full()
                .gap_2()
                .justify_between()
                .child(
                    super::selectable_text(
                        format!("provider-{kind:?}-{name}"),
                        format!(
                            "{kind_label} · {} · {} · {} 条 · 更新于 {}",
                            provider.name,
                            provider.vehicle,
                            provider.item_count,
                            provider.updated_at
                        ),
                    )
                    .text_sm()
                    .text_color(cx.theme().muted_foreground),
                )
                .child(
                    Button::new(format!("update-provider-{kind:?}-{name}"))
                        .label("更新")
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
                "暂无规则",
                "启用配置后点击“刷新”加载规则列表。",
            )
            .action(
                Button::new("empty-refresh-rules")
                    .label("刷新")
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
            h_flex().justify_between().child(page_title("规则")).child(
                Button::new("refresh-rules")
                    .label("刷新")
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
                .child(muted("规则", cx))
                .child(rules),
        )
        .into_any_element()
}
