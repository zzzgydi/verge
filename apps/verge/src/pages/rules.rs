use super::components::PageHeader;
use super::filters::{self, Choice};
use crate::ui::Page;
use crate::{domain::ProviderKind, i18n::tr, ui::UiAction, view::MainView};
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::Scrollbar,
    tooltip::Tooltip,
    v_flex, v_virtual_list,
};
use gpui_kit::*;
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
            Self::Providers | Self::Rules => 48.,
            Self::Provider(_) => 64.,
            _ => ROW_HEIGHT,
        })
    }
}

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let lang = view.lang();
    let refreshing = view.is_pending(&["rules", "providers"]);
    let rule_indices: Vec<_> = view
        .state
        .rules
        .iter()
        .enumerate()
        .filter(|(_, r)| view.filters.rules.matches(r))
        .map(|(i, _)| i)
        .collect();
    let provider_indices: Vec<_> = view
        .state
        .providers
        .iter()
        .enumerate()
        .filter(|(_, p)| view.filters.rules.matches_provider(p))
        .map(|(i, _)| i)
        .collect();
    let rule_count = rule_indices.len();
    let provider_count = provider_indices.len();
    let mut rows = Vec::new();
    if provider_count > 0 {
        rows.push(RuleRow::Providers);
        rows.extend(provider_indices.into_iter().map(RuleRow::Provider));
    }
    if rule_count > 0 {
        rows.push(RuleRow::Rules);
        rows.push(RuleRow::Columns);
        rows.extend(rule_indices.into_iter().map(RuleRow::Rule));
    }
    let empty = rows.is_empty();
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
                                    "{}   {} / {}",
                                    tr(lang, "rules.providers"),
                                    provider_count,
                                    this.state.providers.len()
                                )
                            } else {
                                format!(
                                    "{}   {} / {}",
                                    tr(lang, "rules.section"),
                                    rule_count,
                                    this.state.rules.len()
                                )
                            })
                            .into_any_element(),
                        RuleRow::Provider(ix) => {
                            let provider = &this.state.providers[ix];
                            let name = provider.name.clone();
                            let kind = provider.kind;
                            div()
                                .h(row.height())
                                .pb_2()
                                .child(
                                    base.h(px(56.))
                                        .rounded_lg()
                                        .bg(cx.theme().accent.opacity(0.35))
                                        .justify_between()
                                        .child(
                                            v_flex()
                                                .flex_1()
                                                .min_w_0()
                                                .gap_1()
                                                .child(
                                                    div().text_sm().truncate().child(name.clone()),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(cx.theme().muted_foreground)
                                                        .truncate()
                                                        .child(format!(
                                                            "{} · {} · {} · {}",
                                                            match kind {
                                                                ProviderKind::Rule => tr(
                                                                    lang,
                                                                    "rules.provider_kind.rule"
                                                                ),
                                                                ProviderKind::Proxy => tr(
                                                                    lang,
                                                                    "rules.provider_kind.proxy"
                                                                ),
                                                            },
                                                            provider.vehicle,
                                                            provider.item_count,
                                                            crate::format::provider_updated_at(
                                                                lang,
                                                                &provider.updated_at
                                                            )
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
                                        ),
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
                            .child(div().w(px(28.)))
                            .into_any_element(),
                        RuleRow::Rule(ix) => {
                            let rule = &this.state.rules[ix];
                            let payload = if rule.size > 0 {
                                format!("{} ({})", rule.payload, rule.size)
                            } else {
                                rule.payload.clone()
                            };
                            let target = rule.proxy.clone();
                            let kind = rule.kind.clone();
                            let full_rule =
                                format!("{}, {}, {}", rule.kind, rule.payload, rule.proxy);
                            base.debug_selector(move || format!("rule-row-{ix}"))
                                .text_sm()
                                .border_b_1()
                                .border_color(cx.theme().border.opacity(0.45))
                                .hover(|row| row.bg(cx.theme().list_hover))
                                .child(
                                    div()
                                        .id(("rule-kind", ix))
                                        .tooltip(move |window, cx| {
                                            Tooltip::new(kind.clone()).build(window, cx)
                                        })
                                        .w(px(160.))
                                        .flex_shrink_0()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .truncate()
                                        .child(rule.kind.clone()),
                                )
                                .child(
                                    div()
                                        .id(("rule-payload", ix))
                                        .tooltip(move |window, cx| {
                                            Tooltip::new(payload.clone()).build(window, cx)
                                        })
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
                                        .id(("rule-target", ix))
                                        .tooltip(move |window, cx| {
                                            Tooltip::new(target.clone()).build(window, cx)
                                        })
                                        .w(px(150.))
                                        .flex_shrink_0()
                                        .truncate()
                                        .child(rule.proxy.clone()),
                                )
                                .child(
                                    Button::new(("copy-rule", ix))
                                        .small()
                                        .ghost()
                                        .w(px(28.))
                                        .h(px(28.))
                                        .icon(gpui_kit::component::IconName::Copy)
                                        .tooltip(tr(lang, "common.copy"))
                                        .accessibility_label(format!(
                                            "{}: {}",
                                            tr(lang, "common.copy"),
                                            full_rule
                                        ))
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                full_rule.clone(),
                                            ))
                                        }),
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
            PageHeader::new(tr(lang, "rules.title"))
                .child(filters::count(rule_count, view.state.rules.len(), cx))
                .child(
                    Button::new("refresh-rules")
                        .debug_selector(|| "refresh-rules".into())
                        .tooltip(tr(lang, "common.refresh"))
                        .accessibility_label(tr(lang, "common.refresh"))
                        .icon(gpui_kit::assets::IconName::RefreshCw)
                        .small()
                        .h(px(crate::appearance::metrics::COMPACT_CONTROL))
                        .ghost()
                        .loading(refreshing)
                        .on_click(
                            cx.listener(|this, _, _, cx| this.dispatch(UiAction::RefreshRules, cx)),
                        ),
                ),
        )
        .child(toolbar(view, cx))
        .child(if empty {
            if refreshing && view.state.rules.is_empty() && view.state.providers.is_empty() {
                super::skeleton_rows(5).into_any_element()
            } else {
                super::EmptyState::new(
                    gpui_kit::component::IconName::BookOpen,
                    tr(
                        lang,
                        if view.filters.active(Page::Rules) {
                            "filters.empty"
                        } else {
                            "rules.empty.title"
                        },
                    ),
                    tr(
                        lang,
                        if view.filters.active(Page::Rules) {
                            "filters.empty.desc"
                        } else {
                            "rules.empty.desc"
                        },
                    ),
                )
                .into_any_element()
            }
        } else {
            div()
                .debug_selector(|| "rule-list".into())
                .relative()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .child(list)
                .child(Scrollbar::vertical(&view.rule_scroll))
                .into_any_element()
        })
        .into_any_element()
}

fn toolbar(view: &MainView, cx: &mut Context<MainView>) -> impl IntoElement {
    h_flex()
        .debug_selector(|| "rules-filters".into())
        .gap_2()
        .h_8()
        .child(filters::search(&view.filters.rule_search, "rules-search"))
        .child(filters::choice(
            view,
            Choice {
                id: "rules-type",
                label: "rules.column.type",
                page: Page::Rules,
                selected: view.filters.rules.kind.clone(),
                options: filters::choices(view.state.rules.iter().map(|r| r.kind.clone())),
                set: |v, value| v.filters.rules.kind = value,
            },
            cx,
        ))
        .child(filters::choice(
            view,
            Choice {
                id: "rules-target",
                label: "rules.column.target",
                page: Page::Rules,
                selected: view.filters.rules.target.clone(),
                options: filters::choices(view.state.rules.iter().map(|r| r.proxy.clone())),
                set: |v, value| v.filters.rules.target = value,
            },
            cx,
        ))
        .child(filters::clear_button(view, Page::Rules, cx))
}
