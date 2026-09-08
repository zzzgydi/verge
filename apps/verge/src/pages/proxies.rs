mod model;
mod rows;
#[cfg(test)]
mod tests;

use super::components::{mode_selector, page_heading};
use crate::{
    domain::{AppError, ProfileId, ProxySnapshot, RunMode},
    i18n::{Lang, tr},
    ui::{UiAction, UiState},
    view::MainView,
};
use gpui::{prelude::FluentBuilder as _, *};
use gpui_component::{
    IconName, VirtualListScrollHandle,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::Scrollbar,
    v_flex, v_virtual_list,
};
use model::Row;
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

/// Retained page state. Only visible cards are constructed by the virtual list.
pub struct ProxyPage {
    actions: WeakEntity<MainView>,
    snapshot: Arc<ProxySnapshot>,
    mode: Option<RunMode>,
    profile: Option<ProfileId>,
    revision: u64,
    lang: Lang,
    expanded: HashSet<String>,
    pub search: Entity<InputState>,
    query: String,
    filter_collapsed: HashSet<String>,
    pub scroll: VirtualListScrollHandle,
    rows: Rc<Vec<Row>>,
    sizes: Rc<Vec<Size<Pixels>>>,
    columns: usize,
    delays: HashMap<String, u32>,
    delay_errors: HashMap<String, AppError>,
    pending: HashSet<String>,
    _subscriptions: Vec<Subscription>,
}

impl ProxyPage {
    pub fn new(actions: WeakEntity<MainView>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(tr(Lang::En, "proxies.search")));
        let subscription = cx.subscribe(&search, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.query = input.read(cx).value().to_string();
                this.filter_collapsed.clear();
                this.rebuild();
                this.scroll.set_offset(point(px(0.), px(0.)));
                cx.notify();
            }
        });
        let bounds = cx.observe_window_bounds(window, |this, window, cx| {
            let columns = Self::columns(window);
            if columns != this.columns {
                this.columns = columns;
                this.rebuild();
                cx.notify();
            }
        });
        Self {
            actions,
            snapshot: Arc::default(),
            mode: None,
            profile: None,
            revision: 0,
            lang: Lang::En,
            expanded: HashSet::new(),
            search,
            query: String::new(),
            filter_collapsed: HashSet::new(),
            scroll: VirtualListScrollHandle::new(),
            rows: Rc::default(),
            sizes: Rc::default(),
            columns: Self::columns(window),
            delays: HashMap::new(),
            delay_errors: HashMap::new(),
            pending: HashSet::new(),
            _subscriptions: vec![subscription, bounds],
        }
    }

    fn columns(window: &Window) -> usize {
        if window.viewport_size().width >= px(1080.) {
            2
        } else {
            1
        }
    }

    pub fn sync(&mut self, state: &UiState, lang: Lang, cx: &mut Context<Self>) {
        let layout_changed = !Arc::ptr_eq(&self.snapshot, &state.proxies)
            || self.mode != state.mode
            || self.profile != state.selected_profile;
        let changed = layout_changed
            || self.revision != state.proxy_revision
            || self.lang != lang
            || self.pending != state.delay_pending;
        if !changed {
            return;
        }
        if self.profile != state.selected_profile {
            self.expanded.clear();
        }
        if self.mode != state.mode || self.profile != state.selected_profile {
            self.scroll.set_offset(point(px(0.), px(0.)));
        }
        self.snapshot = state.proxies.clone();
        self.mode = state.mode;
        self.profile = state.selected_profile.clone();
        self.revision = state.proxy_revision;
        self.lang = lang;
        self.delays.clone_from(&state.delays);
        self.delay_errors.clone_from(&state.delay_errors);
        self.pending.clone_from(&state.delay_pending);
        self.expanded
            .retain(|name| self.snapshot.groups.iter().any(|g| &g.name == name));
        if layout_changed {
            self.rebuild();
        }
        cx.notify();
    }

    fn rebuild(&mut self) {
        let rows = model::rows(
            &self.snapshot,
            self.mode,
            &self.expanded,
            &self.query,
            &self.filter_collapsed,
            self.columns,
        );
        self.sizes = Rc::new(
            rows.iter()
                .map(|row| size(px(0.), px(row.height())))
                .collect(),
        );
        self.rows = Rc::new(rows);
    }

    fn toggle(&mut self, group: usize, cx: &mut Context<Self>) {
        let name = self.snapshot.groups[group].name.clone();
        if !self.query.trim().is_empty() {
            if !self.filter_collapsed.remove(&name) {
                self.filter_collapsed.insert(name);
            }
        } else if !self.expanded.remove(&name) {
            self.expanded.insert(name);
        }
        self.rebuild();
        cx.notify();
    }

    fn locate(&mut self, group: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.snapshot.groups[group].selected.is_none() {
            return;
        }
        self.query.clear();
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.expanded
            .insert(self.snapshot.groups[group].name.clone());
        self.rebuild();
        if let Some(ix) = model::selected_row(&self.snapshot, &self.rows, group) {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
        cx.notify();
    }

    fn dispatch(&self, action: UiAction, cx: &mut Context<Self>) {
        let actions = self.actions.clone();
        cx.defer(move |cx| {
            let _ = actions.update(cx, |view, cx| view.dispatch(action, cx));
        });
    }
}

impl Render for ProxyPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let global = self.mode == Some(RunMode::Global);
        let direct = self.mode == Some(RunMode::Direct);
        let global_index = self
            .snapshot
            .groups
            .iter()
            .position(|group| group.name == "GLOBAL");
        let toolbar = h_flex()
            .gap_3()
            .flex_shrink_0()
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.search)
                        .cleanable(true)
                        .prefix(gpui_component::Icon::new(IconName::Search).size_4()),
                ),
            )
            .when(!global && !direct, |this| {
                this.child(
                    Button::new("collapse-proxies")
                        .debug_selector(|| "collapse-proxies".into())
                        .label(tr(self.lang, "proxies.collapse_all"))
                        .h(px(36.))
                        .ghost()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.expanded.clear();
                            this.query.clear();
                            this.search
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            this.rebuild();
                            this.scroll.set_offset(point(px(0.), px(0.)));
                            cx.notify();
                        })),
                )
            })
            .when_some(global_index.filter(|_| global), |this, ix| {
                this.child(
                    Button::new("locate-global")
                        .label(tr(self.lang, "proxies.locate"))
                        .h(px(36.))
                        .outline()
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.locate(ix, window, cx)),
                        ),
                )
            });
        let body = if direct {
            super::EmptyState::new(
                IconName::Globe,
                tr(self.lang, "proxies.direct.title"),
                tr(self.lang, "proxies.direct.desc"),
            )
            .into_any_element()
        } else if self.rows.is_empty() {
            super::EmptyState::new(
                IconName::Search,
                tr(self.lang, "proxies.no_match"),
                tr(self.lang, "proxies.no_match.desc"),
            )
            .into_any_element()
        } else {
            let rows = self.rows.clone();
            div()
                .debug_selector(|| "proxy-list".into())
                .relative()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .child(
                    v_virtual_list(
                        cx.entity(),
                        "proxy-list",
                        self.sizes.clone(),
                        move |this, range, _, cx| {
                            range
                                .filter_map(|ix| rows.get(ix))
                                .map(|row| rows::render(row, this, cx))
                                .collect()
                        },
                    )
                    .track_scroll(&self.scroll),
                )
                .child(Scrollbar::vertical(&self.scroll))
                .into_any_element()
        };
        v_flex()
            .size_full()
            .min_h_0()
            .gap_4()
            .child(toolbar)
            .child(body)
    }
}

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let lang = view.lang();
    v_flex()
        .flex_1()
        .min_h_0()
        .gap_5()
        .child(
            h_flex()
                .justify_between()
                .flex_shrink_0()
                .child(page_heading(
                    tr(lang, "proxies.title"),
                    tr(lang, "proxies.subtitle"),
                    cx,
                ))
                .child(
                    h_flex().gap_3().child(mode_selector(view, cx)).child(
                        Button::new("refresh-proxies")
                            .label(tr(lang, "common.refresh"))
                            .ghost()
                            .loading(view.is_pending(&["proxy_groups"]))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.dispatch(UiAction::RefreshProxies, cx)
                            })),
                    ),
                ),
        )
        .child(view.proxy_page.clone())
        .into_any_element()
}
