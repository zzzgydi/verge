//! Local display filters. No filter changes subscriptions or backend data.
use std::collections::BTreeSet;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
};
use gpui_kit::*;

use crate::{
    domain::{Connection, LogEvent, ProviderSummary, RuleEntry},
    i18n::{Lang, tr},
    ui::Page,
    view::MainView,
};

#[cfg(test)]
mod tests;

/// Every whitespace-separated term must occur in at least one field.
fn includes(query: &str, fields: &[&str]) -> bool {
    if query.trim().is_empty() {
        return true;
    }
    let fields: Vec<_> = fields.iter().map(|s| s.to_lowercase()).collect();
    query.split_whitespace().all(|term| {
        let term = term.to_lowercase();
        fields.iter().any(|field| field.contains(&term))
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuleFilter {
    pub query: String,
    pub kind: Option<String>,
    pub target: Option<String>,
}
impl RuleFilter {
    pub fn matches(&self, rule: &RuleEntry) -> bool {
        self.kind.as_ref().is_none_or(|v| v == &rule.kind)
            && self.target.as_ref().is_none_or(|v| v == &rule.proxy)
            && includes(&self.query, &[&rule.kind, &rule.payload, &rule.proxy])
    }
    pub fn matches_provider(&self, provider: &ProviderSummary) -> bool {
        self.kind.is_none()
            && self.target.is_none()
            && includes(&self.query, &[&provider.name, &provider.vehicle])
    }
    fn active(&self) -> bool {
        !self.query.trim().is_empty() || self.kind.is_some() || self.target.is_some()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConnectionFilter {
    pub query: String,
    pub network: Option<String>,
    pub process: Option<String>,
    pub chain: Option<String>,
}
impl ConnectionFilter {
    pub fn matches(&self, row: &Connection) -> bool {
        self.network
            .as_ref()
            .is_none_or(|v| row.network.eq_ignore_ascii_case(v))
            && self.process.as_ref().is_none_or(|v| v == &row.process)
            && self
                .chain
                .as_ref()
                .is_none_or(|v| row.chains.contains(v) || (row.chains.is_empty() && v == "DIRECT"))
            && includes(
                &self.query,
                &[
                    &row.process,
                    &row.host,
                    &row.destination,
                    &row.source,
                    &row.network,
                    &row.rule,
                    &row.rule_payload,
                    &row.chains.join(" "),
                ],
            )
    }
    fn active(&self) -> bool {
        !self.query.trim().is_empty()
            || self.network.is_some()
            || self.process.is_some()
            || self.chain.is_some()
    }
}

pub fn normalized_level(level: &str) -> String {
    match level.to_ascii_lowercase().as_str() {
        "warn" => "warning".into(),
        other => other.into(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogFilter {
    pub query: String,
    pub exclude: String,
    pub level: Option<String>,
}
impl LogFilter {
    pub fn matches(&self, row: &LogEvent) -> bool {
        let level = normalized_level(&row.level);
        self.level.as_ref().is_none_or(|value| {
            if value == "problems" {
                matches!(level.as_str(), "warning" | "error" | "fatal" | "panic")
            } else {
                value == &level
            }
        }) && includes(&self.query, &[&row.payload])
            && !self
                .exclude
                .split_whitespace()
                .any(|word| includes(word, &[&row.payload]))
    }
    fn active(&self) -> bool {
        !self.query.trim().is_empty() || !self.exclude.trim().is_empty() || self.level.is_some()
    }
}

pub struct PageFilters {
    pub rules: RuleFilter,
    pub connections: ConnectionFilter,
    pub logs: LogFilter,
    pub rule_search: Entity<InputState>,
    pub connection_search: Entity<InputState>,
    pub log_search: Entity<InputState>,
    pub log_exclude: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}
impl PageFilters {
    pub fn new(window: &mut Window, cx: &mut Context<MainView>) -> Self {
        let inputs = [
            "rules.search",
            "connections.search",
            "logs.search",
            "logs.exclude",
        ]
        .map(|key| cx.new(|cx| InputState::new(window, cx).placeholder(tr(Lang::En, key))));
        let subscriptions = inputs
            .iter()
            .enumerate()
            .map(|(index, input)| {
                cx.subscribe(input, move |view, input, event: &InputEvent, cx| {
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    let query = input.read(cx).value().to_string();
                    let page = match index {
                        0 => {
                            view.filters.rules.query = query;
                            Page::Rules
                        }
                        1 => {
                            view.filters.connections.query = query;
                            Page::Connections
                        }
                        2 => {
                            view.filters.logs.query = query;
                            Page::Logs
                        }
                        _ => {
                            view.filters.logs.exclude = query;
                            Page::Logs
                        }
                    };
                    changed(view, page, cx);
                })
            })
            .collect();
        let [rule_search, connection_search, log_search, log_exclude] = inputs;
        Self {
            rules: Default::default(),
            connections: Default::default(),
            logs: Default::default(),
            rule_search,
            connection_search,
            log_search,
            log_exclude,
            _subscriptions: subscriptions,
        }
    }
    pub fn active(&self, page: Page) -> bool {
        match page {
            Page::Rules => self.rules.active(),
            Page::Connections => self.connections.active(),
            Page::Logs => self.logs.active(),
            _ => false,
        }
    }
}

fn changed(view: &mut MainView, page: Page, cx: &mut Context<MainView>) {
    match page {
        Page::Rules => view.rule_scroll.set_offset(point(px(0.), px(0.))),
        Page::Logs => view.log_scroll.set_offset(point(px(0.), px(0.))),
        Page::Connections => view.sync_connections(cx),
        _ => {}
    }
    cx.notify();
}

pub fn clear(view: &mut MainView, page: Page, window: &mut Window, cx: &mut Context<MainView>) {
    let inputs = match page {
        Page::Rules => {
            view.filters.rules = Default::default();
            vec![view.filters.rule_search.clone()]
        }
        Page::Connections => {
            view.filters.connections = Default::default();
            vec![view.filters.connection_search.clone()]
        }
        Page::Logs => {
            view.filters.logs = Default::default();
            vec![
                view.filters.log_search.clone(),
                view.filters.log_exclude.clone(),
            ]
        }
        _ => vec![],
    };
    for input in inputs {
        input.update(cx, |input, cx| input.set_value("", window, cx));
    }
    changed(view, page, cx);
}

pub fn search(input: &Entity<InputState>, id: &'static str) -> impl IntoElement {
    div()
        .debug_selector(move || id.into())
        .flex_1()
        .min_w(px(110.))
        .child(Input::new(input).small().cleanable(true))
}

pub fn count(visible: usize, total: usize, cx: &Context<MainView>) -> impl IntoElement {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(format!("{visible} / {total}"))
}

pub fn clear_button(view: &MainView, page: Page, cx: &mut Context<MainView>) -> Button {
    let id = match page {
        Page::Rules => "clear-rules-filter",
        Page::Connections => "clear-connections-filter",
        _ => "clear-log-filter",
    };
    Button::new(id)
        .debug_selector(move || id.into())
        .small()
        .ghost()
        .label(tr(view.lang(), "filters.clear"))
        .disabled(!view.filters.active(page))
        .on_click(cx.listener(move |view, _, window, cx| clear(view, page, window, cx)))
}

pub fn choices(values: impl IntoIterator<Item = String>) -> Vec<(String, String)> {
    values
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|v| (v.clone(), v))
        .collect()
}

pub struct Choice {
    pub id: &'static str,
    pub label: &'static str,
    pub page: Page,
    pub selected: Option<String>,
    pub options: Vec<(String, String)>,
    pub set: fn(&mut MainView, Option<String>),
}
pub fn choice(view: &MainView, mut choice: Choice, cx: &mut Context<MainView>) -> impl IntoElement {
    let lang = view.lang();
    // Keep a vanished live value visible until the user explicitly clears it.
    if let Some(selected) = &choice.selected
        && !choice.options.iter().any(|(value, _)| value == selected)
    {
        let label = if choice.id == "connections-process" && selected.is_empty() {
            tr(lang, "connections.unknown_process").to_owned()
        } else {
            selected.clone()
        };
        choice.options.push((selected.clone(), label));
    }
    let value = choice
        .selected
        .as_ref()
        .and_then(|v| choice.options.iter().find(|(key, _)| key == v))
        .map_or(tr(lang, "filters.all"), |(_, label)| label.as_str());
    let label = if choice.page == Page::Logs {
        crate::i18n::fmt_logs_filter(lang, value)
    } else {
        format!("{}: {value}", tr(lang, choice.label))
    };
    let entity = cx.entity().downgrade();
    let id = choice.id;
    Button::new(id)
        .debug_selector(move || id.into())
        .small()
        .outline()
        .max_w(px(145.))
        .dropdown_caret(true)
        .tooltip(label.clone())
        .label(label)
        .dropdown_menu(move |menu, _, _| {
            let options = std::iter::once((None, tr(lang, "filters.all").to_owned())).chain(
                choice
                    .options
                    .iter()
                    .map(|(v, label)| (Some(v.clone()), label.clone())),
            );
            options.fold(
                menu.scrollable(true).max_h(px(300.)),
                |menu, (value, label)| {
                    let entity = entity.clone();
                    menu.item(
                        PopupMenuItem::new(label)
                            .checked(value == choice.selected)
                            .on_click(move |_, _, cx| {
                                let _ = entity.update(cx, |view, cx| {
                                    (choice.set)(view, value.clone());
                                    changed(view, choice.page, cx);
                                });
                            }),
                    )
                },
            )
        })
}
