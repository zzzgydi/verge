mod model;
mod rows;
#[cfg(test)]
mod tests;
mod tools;

use super::components::{PageHeader, mode_selector};
use crate::{
    domain::{AppError, ProfileId, ProxySnapshot, RunMode},
    i18n::{Lang, tr},
    ui::{UiAction, UiState},
    view::MainView,
};
use gpui_kit::component::{
    IconName, Sizable as _, VirtualListScrollHandle,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::Scrollbar,
    v_flex, v_virtual_list,
};
use gpui_kit::{prelude::FluentBuilder as _, *};
use model::Row;
use std::{
    collections::{HashMap, HashSet, VecDeque},
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
    node_focus: FocusHandle,
    cursor: Option<(usize, usize)>,
    delays: HashMap<String, u32>,
    delay_errors: HashMap<String, AppError>,
    pending: HashSet<String>,
    group_filters: HashMap<String, tools::GroupFilter>,
    group_searches: HashMap<String, Entity<InputState>>,
    group_subscriptions: HashMap<String, Subscription>,
    search_lang: Lang,
    animation: HashMap<String, f32>,
    animation_task: Option<Task<()>>,
    test_queue: VecDeque<String>,
    test_reserved: HashSet<String>,
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
            node_focus: cx.focus_handle(),
            cursor: None,
            delays: HashMap::new(),
            delay_errors: HashMap::new(),
            pending: HashSet::new(),
            group_filters: HashMap::new(),
            group_searches: HashMap::new(),
            group_subscriptions: HashMap::new(),
            search_lang: Lang::En,
            animation: HashMap::new(),
            animation_task: None,
            test_queue: VecDeque::new(),
            test_reserved: HashSet::new(),
            _subscriptions: vec![subscription, bounds],
        }
    }

    fn keyboard_nodes(&self) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
        self.rows.iter().enumerate().flat_map(move |(row, entry)| {
            let (group, members) = match entry {
                Row::Nodes { group, members } => (*group, members.as_slice()),
                _ => (0, [].as_slice()),
            };
            members
                .iter()
                .copied()
                .filter(move |&member| {
                    let entry = &self.snapshot.groups[group];
                    matches!(entry.kind.as_str(), "Selector" | "URLTest" | "Fallback")
                        && self.snapshot.proxies.contains_key(&entry.members[member])
                })
                .map(move |member| (row, group, member))
        })
    }

    fn keyboard_cursor(&self) -> Option<(usize, usize)> {
        let top = -self.scroll.offset().y;
        let bottom = top + self.scroll.bounds().size.height;
        let mut y = px(0.);
        let mut first = self.rows.len();
        let mut end = self.rows.len();
        for (index, size) in self.sizes.iter().enumerate() {
            if y >= bottom {
                end = index;
                break;
            }
            if y + size.height > top && first == self.rows.len() {
                first = index;
            }
            y += size.height;
        }
        let visible = |row: usize| (first..end).contains(&row);
        let current = self.cursor.and_then(|cursor| {
            self.keyboard_nodes()
                .find(|&(row, group, member)| (group, member) == cursor && visible(row))
        });
        current
            .or_else(|| self.keyboard_nodes().find(|&(row, _, _)| visible(row)))
            .map(|(_, group, member)| (group, member))
    }

    fn move_cursor(
        &mut self,
        group: usize,
        member: usize,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let nodes: Vec<_> = self.keyboard_nodes().collect();
        let Some(index) = nodes
            .iter()
            .position(|(_, g, m)| (*g, *m) == (group, member))
        else {
            return;
        };
        let step = if matches!(key, "up" | "down") {
            self.columns
        } else {
            1
        };
        let next = if matches!(key, "up" | "left") {
            index.saturating_sub(step)
        } else {
            (index + step).min(nodes.len() - 1)
        };
        let (row, group, member) = nodes[next];
        self.cursor = Some((group, member));
        let top = self
            .sizes
            .iter()
            .take(row)
            .map(|size| size.height)
            .sum::<Pixels>();
        let bottom = top + self.sizes[row].height;
        let viewport_top = -self.scroll.offset().y;
        let height = self.scroll.bounds().size.height;
        if top < viewport_top {
            self.scroll.set_offset(point(px(0.), -top));
        } else if bottom > viewport_top + height {
            self.scroll.set_offset(point(px(0.), height - bottom));
        }
        self.node_focus.focus(window, cx);
        cx.notify();
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
        if layout_changed {
            self.cursor = None;
        }
        if self.profile != state.selected_profile {
            self.expanded.clear();
            self.group_filters.clear();
            self.group_searches.clear();
            self.group_subscriptions.clear();
            self.animation.clear();
            self.test_queue.clear();
        }
        if self.mode != state.mode || self.profile != state.selected_profile {
            self.scroll.set_offset(point(px(0.), px(0.)));
        }
        if self.mode != state.mode || state.connection_notice.is_some() {
            self.animation.clear();
            self.animation_task = None;
            self.test_queue.clear();
        }
        self.snapshot = state.proxies.clone();
        self.group_searches
            .retain(|name, _| self.snapshot.groups.iter().any(|g| &g.name == name));
        self.group_subscriptions
            .retain(|name, _| self.group_searches.contains_key(name));
        self.group_filters
            .retain(|name, _| self.group_searches.contains_key(name));
        self.test_queue
            .retain(|name| self.snapshot.proxies.contains_key(name));
        self.mode = state.mode;
        self.profile = state.selected_profile.clone();
        self.revision = state.proxy_revision;
        self.lang = lang;
        self.delays.clone_from(&state.delays);
        self.delay_errors.clone_from(&state.delay_errors);
        self.test_reserved
            .retain(|name| !state.delay_pending.contains(name));
        self.pending.clone_from(&state.delay_pending);
        self.pending.extend(self.test_reserved.iter().cloned());
        self.expanded
            .retain(|name| self.snapshot.groups.iter().any(|g| &g.name == name));
        self.rebuild();
        self.pump_tests(cx);
        cx.notify();
    }

    fn rebuild(&mut self) {
        let mut visible = self.expanded.clone();
        visible.extend(self.animation.keys().cloned());
        let collapsed = self
            .filter_collapsed
            .iter()
            .filter(|name| !self.animation.contains_key(*name))
            .cloned()
            .collect();
        let rows = model::rows(
            &self.snapshot,
            self.mode,
            &visible,
            &self.query,
            &collapsed,
            self.columns,
        );
        let rows = self.filter_rows(rows);
        self.sizes = Rc::new(
            rows.iter()
                .map(|row| size(px(0.), px(self.row_height(row))))
                .collect(),
        );
        self.rows = Rc::new(rows);
    }

    fn is_expanded(&self, name: &str) -> bool {
        if self.query.trim().is_empty() {
            self.expanded.contains(name)
        } else {
            !self.filter_collapsed.contains(name)
        }
    }

    fn row_height(&self, row: &Row) -> f32 {
        let group = match row {
            Row::Group(_) => return row.height(),
            Row::Toolbar(g) | Row::Nodes { group: g, .. } => *g,
        };
        row.height()
            * self
                .animation
                .get(&self.snapshot.groups[group].name)
                .copied()
                .unwrap_or(1.)
    }

    fn toggle(&mut self, group: usize, cx: &mut Context<Self>) {
        let name = self.snapshot.groups[group].name.clone();
        let was_open = self.is_expanded(&name);
        if !self.query.trim().is_empty() {
            if !self.filter_collapsed.remove(&name) {
                self.filter_collapsed.insert(name.clone());
            }
        } else if !self.expanded.remove(&name) {
            self.expanded.insert(name.clone());
        }
        if !cx.reduce_motion() {
            self.animation
                .entry(name)
                .or_insert(if was_open { 1. } else { 0.01 });
            self.animate(cx);
        }
        self.rebuild();
        cx.notify();
    }

    fn animate(&mut self, cx: &mut Context<Self>) {
        self.animation_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(16))
                    .await;
                let Ok(active) = this.update(cx, |this, cx| {
                    let names: Vec<_> = this.animation.keys().cloned().collect();
                    for name in names {
                        let target = if this.is_expanded(&name) { 1. } else { 0. };
                        let progress = this.animation.get_mut(&name).unwrap();
                        *progress += (target - *progress).clamp(-0.1, 0.1);
                        if (*progress - target).abs() < 0.001 {
                            this.animation.remove(&name);
                        }
                    }
                    this.rebuild();
                    cx.notify();
                    !this.animation.is_empty()
                }) else {
                    break;
                };
                if !active {
                    break;
                }
            }
        }));
    }

    fn locate(&mut self, group: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.snapshot.groups[group].selected.is_none() {
            return;
        }
        self.animation.clear();
        let name = self.snapshot.groups[group].name.clone();
        self.group_filters.remove(&name);
        if let Some(input) = self.group_searches.get(&name) {
            input.update(cx, |input, cx| input.set_value("", window, cx));
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_group_searches(window, cx);
        let entity = cx.entity().downgrade();
        let current_columns = self.columns;
        let measure_width = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let width = bounds.size.width;
                let columns = if width >= px(780.) { 2 } else { 1 };
                if current_columns == columns {
                    return;
                }
                let entity = entity.clone();
                window.on_next_frame(move |_, cx| {
                    let _ = entity.update(cx, |page, cx| {
                        if page.columns != columns {
                            page.columns = columns;
                            page.rebuild();
                            cx.notify();
                        }
                    });
                });
            },
        )
        .absolute()
        .size_full();
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
                        .prefix(gpui_kit::component::Icon::new(IconName::Search).size_4()),
                ),
            )
            .when(!global && !direct, |this| {
                this.child(
                    Button::new("collapse-proxies")
                        .small()
                        .debug_selector(|| "collapse-proxies".into())
                        .label(tr(self.lang, "proxies.collapse_all"))
                        .h(px(crate::appearance::metrics::CONTROL))
                        .ghost()
                        .on_click(cx.listener(|this, _, window, cx| {
                            let opened: Vec<_> = this
                                .snapshot
                                .groups
                                .iter()
                                .filter(|g| this.is_expanded(&g.name))
                                .map(|g| g.name.clone())
                                .collect();
                            if !cx.reduce_motion() {
                                for name in opened {
                                    this.animation.entry(name).or_insert(1.);
                                }
                                this.animate(cx);
                            }
                            this.expanded.clear();
                            this.query.clear();
                            this.search
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            this.rebuild();
                            this.scroll.set_offset(point(px(0.), px(0.)));
                            cx.notify();
                        })),
                )
            });
        let toolbar = if let Some(index) = global_index.filter(|_| global) {
            tools::render(index, self, cx)
        } else {
            toolbar.into_any_element()
        };
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
                                .map(|row| {
                                    div()
                                        .h(px(this.row_height(row)))
                                        .overflow_hidden()
                                        .child(rows::render(row, this, cx))
                                        .into_any_element()
                                })
                                .collect()
                        },
                    )
                    .track_scroll(&self.scroll),
                )
                .child(Scrollbar::vertical(&self.scroll))
                .into_any_element()
        };
        v_flex()
            .relative()
            .size_full()
            .min_h_0()
            .gap_4()
            .child(measure_width)
            .child(toolbar)
            .child(body)
    }
}

pub fn render(view: &MainView, cx: &mut Context<MainView>) -> AnyElement {
    let lang = view.lang();
    v_flex()
        .flex_1()
        .min_h_0()
        .gap_4()
        .child(
            PageHeader::new(tr(lang, "proxies.title")).child(
                h_flex().gap_3().child(mode_selector(view, cx)).child(
                    Button::new("refresh-proxies")
                        .small()
                        .h(px(crate::appearance::metrics::COMPACT_CONTROL))
                        .icon(gpui_kit::assets::IconName::RefreshCw)
                        .tooltip(tr(lang, "common.refresh"))
                        .accessibility_label(tr(lang, "common.refresh"))
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
