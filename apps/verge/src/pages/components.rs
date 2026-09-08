//! Value-like visual components; retained behavior belongs to feature entities.
use crate::appearance::metrics;
use gpui::*;
use gpui_component::{ActiveTheme as _, Icon, IconName, StyledExt as _, h_flex, v_flex};

#[derive(IntoElement)]
pub struct Metric {
    label: SharedString,
    value: SharedString,
    icon: IconName,
}

impl Metric {
    pub fn new(
        label: impl Into<SharedString>,
        value: impl Into<SharedString>,
        icon: IconName,
    ) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            icon,
        }
    }
}

impl RenderOnce for Metric {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        panel(cx)
            .flex_1()
            .min_w_0()
            .gap_3()
            .child(
                gpui_component::h_flex()
                    .justify_between()
                    .text_color(cx.theme().muted_foreground)
                    .text_xs()
                    .child(self.label)
                    .child(Icon::new(self.icon).size_4()),
            )
            .child(div().text_2xl().font_medium().child(self.value))
    }
}

pub fn panel(cx: &App) -> Div {
    v_flex()
        .p(px(metrics::PANEL_INSET))
        .gap(px(metrics::SECTION_GAP))
        .bg(cx.theme().tiles)
        .border_1()
        .border_color(cx.theme().border)
        .rounded(px(metrics::PANEL_RADIUS))
}

/// Single-line title with optional trailing actions. No reserved subtitle space.
#[derive(IntoElement)]
pub struct PageHeader {
    title: SharedString,
    actions: Vec<AnyElement>,
}

impl PageHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            actions: Vec::new(),
        }
    }
}

impl ParentElement for PageHeader {
    fn extend(&mut self, children: impl IntoIterator<Item = AnyElement>) {
        self.actions.extend(children);
    }
}

impl RenderOnce for PageHeader {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        h_flex()
            .debug_selector(|| "page-header".into())
            .w_full()
            .h(px(metrics::CONTROL))
            .flex_shrink_0()
            .gap_4()
            .justify_between()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(metrics::PAGE_TITLE))
                    .line_height(px(metrics::CONTROL))
                    .font_semibold()
                    .child(self.title),
            )
            .child(h_flex().flex_shrink_0().gap_2().children(self.actions))
    }
}

pub fn mode_selector(
    view: &crate::view::MainView,
    cx: &mut Context<crate::view::MainView>,
) -> impl IntoElement {
    use crate::{domain::RunMode, ui::UiAction};
    use gpui_component::{
        Disableable as _, Selectable as _, Sizable as _,
        button::{Button, ButtonGroup},
    };
    const MODES: [RunMode; 3] = [RunMode::Rule, RunMode::Global, RunMode::Direct];
    ButtonGroup::new("mode-group")
        .small()
        .outline()
        .children(MODES.map(|mode| {
            Button::new(format!("mode-{mode:?}"))
                .debug_selector(move || format!("mode-{mode:?}"))
                .label(super::home::mode_label(view.lang(), mode))
                .min_w(px(64.))
                .h(px(metrics::CONTROL))
                .disabled(view.state.mode.is_none() || view.is_pending(&["mode"]))
                .selected(view.state.mode == Some(mode))
        }))
        .on_click(cx.listener(|this, clicked: &Vec<usize>, _, cx| {
            if let Some(&ix) = clicked.first() {
                this.dispatch(UiAction::SetMode(MODES[ix]), cx);
            }
        }))
}
