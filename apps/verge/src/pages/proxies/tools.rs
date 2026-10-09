use super::{ProxyPage, model::Row};
use crate::{i18n::tr, ui::UiAction};
use gpui_kit::component::{
    Disableable as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
};
use gpui_kit::{prelude::FluentBuilder as _, *};

#[derive(Default)]
pub(super) struct GroupFilter {
    pub(super) query: String,
    sort: u8,
    available_only: bool,
}

impl ProxyPage {
    pub(super) fn ensure_group_searches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_lang != self.lang {
            for input in self.group_searches.values() {
                input.update(cx, |input, cx| {
                    input.set_placeholder(tr(self.lang, "proxies.filter_nodes"), window, cx)
                });
            }
            self.search_lang = self.lang;
        }
        for group in &self.snapshot.groups {
            if self.group_searches.contains_key(&group.name) {
                continue;
            }
            let input = cx.new(|cx| {
                InputState::new(window, cx).placeholder(tr(self.lang, "proxies.filter_nodes"))
            });
            let name = group.name.clone();
            self.group_subscriptions.insert(
                group.name.clone(),
                cx.subscribe(&input, move |this, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.group_filters.entry(name.clone()).or_default().query =
                            input.read(cx).value().to_string();
                        this.rebuild();
                        cx.notify();
                    }
                }),
            );
            self.group_searches.insert(group.name.clone(), input);
        }
    }

    pub(super) fn filter_rows(&self, rows: Vec<Row>) -> Vec<Row> {
        let mut output = Vec::new();
        let mut rows = rows.into_iter().peekable();
        while let Some(row) = rows.next() {
            let Row::Nodes { group, mut members } = row else {
                output.push(row);
                continue;
            };
            while matches!(rows.peek(), Some(Row::Nodes {group:g,..}) if *g == group) {
                if let Some(Row::Nodes { members: more, .. }) = rows.next() {
                    members.extend(more);
                }
            }
            let entry = &self.snapshot.groups[group];
            if let Some(filter) = self.group_filters.get(&entry.name) {
                let query = filter.query.trim().to_lowercase();
                members.retain(|&ix| {
                    let name = &entry.members[ix];
                    let detail = self.snapshot.proxies.get(name);
                    let delay = self
                        .delays
                        .get(name)
                        .copied()
                        .or_else(|| detail.and_then(|p| p.delay));
                    let matches = name.to_lowercase().contains(&query)
                        || detail.is_some_and(|p| p.kind.to_lowercase().contains(&query));
                    matches
                        && (!filter.available_only
                            || (!self.delay_errors.contains_key(name) && delay != Some(0)))
                });
                if filter.sort != 0 {
                    members.sort_by_key(|&ix| {
                        let name = &entry.members[ix];
                        let delay = self
                            .delays
                            .get(name)
                            .copied()
                            .or_else(|| self.snapshot.proxies.get(name).and_then(|p| p.delay));
                        let delay = delay
                            .filter(|ms| *ms > 0 && !self.delay_errors.contains_key(name))
                            .unwrap_or(u32::MAX);
                        (
                            if filter.sort == 2 { delay } else { 0 },
                            name.to_lowercase(),
                        )
                    });
                }
            }
            output.extend(members.chunks(self.columns).map(|members| Row::Nodes {
                group,
                members: members.to_vec(),
            }));
        }
        output
    }

    pub(super) fn test_all(&mut self, group: usize, cx: &mut Context<Self>) {
        for name in &self.snapshot.groups[group].members {
            if self.snapshot.proxies.contains_key(name)
                && !self.pending.contains(name)
                && !self.test_queue.contains(name)
            {
                self.test_queue.push_back(name.clone());
            }
        }
        self.pump_tests(cx);
        cx.notify();
    }

    pub(super) fn pump_tests(&mut self, cx: &mut Context<Self>) {
        for _ in 0..4_usize.saturating_sub(self.pending.len()) {
            let Some(proxy) = self.test_queue.pop_front() else {
                break;
            };
            self.pending.insert(proxy.clone());
            self.test_reserved.insert(proxy.clone());
            let actions = self.actions.clone();
            let page = cx.entity().downgrade();
            cx.defer(move |cx| {
                let accepted = actions
                    .update(cx, |view, cx| {
                        view.dispatch_sheet_request(
                            UiAction::TestDelay {
                                proxy: proxy.clone(),
                                url: "https://www.gstatic.com/generate_204".into(),
                                timeout_ms: 5_000,
                            },
                            cx,
                        )
                        .is_some()
                    })
                    .unwrap_or(false);
                let _ = page.update(cx, |this, cx| {
                    this.test_reserved.remove(&proxy);
                    if !accepted {
                        this.pending.remove(&proxy);
                        this.test_queue.clear();
                    }
                    cx.notify();
                });
            });
        }
    }
}

pub(super) fn render(group: usize, page: &ProxyPage, cx: &mut Context<ProxyPage>) -> AnyElement {
    let global = page.mode == Some(crate::domain::RunMode::Global);
    let name = page.snapshot.groups[group].name.clone();
    let lang = page.lang;
    let busy = page.snapshot.groups[group]
        .members
        .iter()
        .any(|n| page.pending.contains(n) || page.test_queue.contains(n));
    let filter = page.group_filters.get(&name);
    let sort = filter.map_or(0, |f| f.sort);
    let available = filter.is_some_and(|f| f.available_only);
    let entity = cx.entity().downgrade();
    let sort_name = name.clone();
    h_flex()
        .h(px(44.))
        .pb_2()
        .when(!global, |row| row.pl(px(28.)))
        .gap_2()
        .debug_selector(move || format!("proxy-tools-{group}"))
        .child(
            Button::new(("test-all", group))
                .small()
                .ghost()
                .label(tr(lang, "proxies.test_all"))
                .disabled(busy)
                .on_click(cx.listener(move |this, _, _, cx| this.test_all(group, cx))),
        )
        .child(
            Button::new(("locate-selected", group))
                .small()
                .ghost()
                .label(tr(lang, "proxies.locate"))
                .disabled(page.snapshot.groups[group].selected.is_none())
                .on_click(cx.listener(move |this, _, window, cx| this.locate(group, window, cx))),
        )
        .child(
            div().flex_1().min_w(px(90.)).child(
                Input::new(if global {
                    &page.search
                } else {
                    &page.group_searches[&name]
                })
                .small()
                .cleanable(true),
            ),
        )
        .child(
            Button::new(("node-sort", group))
                .small()
                .outline()
                .label(tr(
                    lang,
                    [
                        "proxies.sort_default",
                        "proxies.sort_name",
                        "proxies.sort_delay",
                    ][sort as usize],
                ))
                .dropdown_menu(move |mut menu, _, _| {
                    for (value, key) in [
                        "proxies.sort_default",
                        "proxies.sort_name",
                        "proxies.sort_delay",
                    ]
                    .iter()
                    .enumerate()
                    {
                        let entity = entity.clone();
                        let name = sort_name.clone();
                        menu = menu.item(PopupMenuItem::new(tr(lang, key)).on_click(
                            move |_, _, cx| {
                                let _ = entity.update(cx, |this, cx| {
                                    this.group_filters.entry(name.clone()).or_default().sort =
                                        value as u8;
                                    this.rebuild();
                                    cx.notify();
                                });
                            },
                        ));
                    }
                    menu
                }),
        )
        .child(
            Button::new(("hide-unavailable", group))
                .small()
                .ghost()
                .selected(available)
                .label(tr(lang, "proxies.hide_unavailable"))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let filter = this.group_filters.entry(name.clone()).or_default();
                    filter.available_only = !filter.available_only;
                    this.rebuild();
                    cx.notify();
                })),
        )
        .into_any_element()
}
