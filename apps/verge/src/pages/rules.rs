use super::components::PageHeader;
use crate::{domain::ProviderKind, i18n::tr, ui::UiAction, view::MainView};
use gpui::*;
use gpui_component::{
    ActiveTheme as _, Sizable as _, StyledExt as _, button::Button, h_flex, scroll::Scrollbar,
    v_flex, v_virtual_list,
};
use std::rc::Rc;

const ROW_HEIGHT: f32 = 36.;
#[derive(Clone, Copy)]
enum RuleRow {
    Providers,
    Provider(usize),
    Rules,
    Columns,
    Rule(usize),
}
impl RuleRow {
    fn height(self) -> Pixels {
        px(match self {
            Self::Providers | Self::Rules => 40.,
            Self::Provider(_) => 56.,
            _ => ROW_HEIGHT,
        })
    }
}

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let lang = view.lang();
    let refreshing = view.is_pending(&["rules", "providers"]);
    if view.state.rules.is_empty() && view.state.providers.is_empty() {
        return v_flex()
            .flex_1()
            .min_h_0()
            .gap_4()
            .child(PageHeader::new(tr(lang, "rules.title")))
            .child(if refreshing {
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
                        .h(px(crate::appearance::metrics::COMPACT_CONTROL))
                        .outline()
                        .on_click(
                            cx.listener(|this, _, _, cx| this.dispatch(UiAction::RefreshRules, cx)),
                        ),
                )
                .into_any_element()
            })
            .into_any_element();
    }
    let mut rows = Vec::new();
    if !view.state.providers.is_empty() {
        rows.push(RuleRow::Providers);
        rows.extend((0..view.state.providers.len()).map(RuleRow::Provider));
    }
    rows.push(RuleRow::Rules);
    if !view.state.rules.is_empty() {
        rows.push(RuleRow::Columns);
        rows.extend((0..view.state.rules.len()).map(RuleRow::Rule));
    }
    let sizes = Rc::new(rows.iter().map(|row| size(px(0.), row.height())).collect());
    let list = v_virtual_list(
        cx.entity(),
        "rule-list",
        sizes,
        move |this, range, _, cx| {
            range
                .map(|ix| {
                    let row = rows[ix];
                    let base = h_flex()
                        .h(row.height())
                        .w_full()
                        .px_3()
                        .gap_3()
                        .overflow_hidden();
                    match row {
                        RuleRow::Providers | RuleRow::Rules => base
                            .font_medium()
                            .text_sm()
                            .child(if matches!(row, RuleRow::Providers) {
                                format!(
                                    "{}   /   {}",
                                    tr(lang, "rules.providers"),
                                    this.state.providers.len()
                                )
                            } else {
                                format!(
                                    "{}   /   {}",
                                    tr(lang, "rules.section"),
                                    this.state.rules.len()
                                )
                            })
                            .into_any_element(),
                        RuleRow::Provider(ix) => {
                            let provider = &this.state.providers[ix];
                            let name = provider.name.clone();
                            let kind = provider.kind;
                            base.rounded_lg()
                                .bg(cx.theme().accent.opacity(0.35))
                                .justify_between()
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap_1()
                                        .child(div().text_sm().truncate().child(name.clone()))
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .truncate()
                                                .child(format!(
                                                    "{} · {} · {} · {}",
                                                    match kind {
                                                        ProviderKind::Rule =>
                                                            tr(lang, "rules.provider_kind.rule"),
                                                        ProviderKind::Proxy =>
                                                            tr(lang, "rules.provider_kind.proxy"),
                                                    },
                                                    provider.vehicle,
                                                    provider.item_count,
                                                    provider.updated_at
                                                )),
                                        ),
                                )
                                .child(
                                    Button::new(format!("update-provider-{kind:?}-{name}"))
                                        .label(tr(lang, "rules.update"))
                                        .small()
                                        .h(px(crate::appearance::metrics::COMPACT_CONTROL))
                                        .outline()
                                        .loading(this.is_pending(&["provider_write"]))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.dispatch(
                                                UiAction::UpdateProvider {
                                                    kind,
                                                    name: name.clone(),
                                                },
                                                cx,
                                            )
                                        })),
                                )
                                .into_any_element()
                        }
                        RuleRow::Columns => base
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(div().w(px(160.)).child(tr(lang, "rules.column.type")))
                            .child(div().flex_1().child(tr(lang, "rules.column.match")))
                            .child(div().w(px(150.)).child(tr(lang, "rules.column.target")))
                            .into_any_element(),
                        RuleRow::Rule(ix) => {
                            let rule = &this.state.rules[ix];
                            base.text_sm()
                                .border_b_1()
                                .border_color(cx.theme().border.opacity(0.45))
                                .hover(|row| row.bg(cx.theme().list_hover))
                                .child(
                                    div()
                                        .w(px(160.))
                                        .flex_shrink_0()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .truncate()
                                        .child(rule.kind.clone()),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .font_family(cx.theme().mono_font_family.clone())
                                        .child(if rule.size > 0 {
                                            format!("{} ({})", rule.payload, rule.size)
                                        } else {
                                            rule.payload.clone()
                                        }),
                                )
                                .child(
                                    div()
                                        .w(px(150.))
                                        .flex_shrink_0()
                                        .truncate()
                                        .child(rule.proxy.clone()),
                                )
                                .into_any_element()
                        }
                    }
                })
                .collect()
        },
    )
    .track_scroll(&view.rule_scroll);
    v_flex()
        .flex_1()
        .min_h_0()
        .gap_4()
        .child(
            PageHeader::new(tr(lang, "rules.title")).child(
                Button::new("refresh-rules")
                    .debug_selector(|| "refresh-rules".into())
                    .label(tr(lang, "common.refresh"))
                    .small()
                    .h(px(crate::appearance::metrics::COMPACT_CONTROL))
                    .outline()
                    .loading(refreshing)
                    .on_click(
                        cx.listener(|this, _, _, cx| this.dispatch(UiAction::RefreshRules, cx)),
                    ),
            ),
        )
        .child(
            div()
                .debug_selector(|| "rule-list".into())
                .relative()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .child(list)
                .child(Scrollbar::vertical(&view.rule_scroll)),
        )
        .into_any_element()
}
