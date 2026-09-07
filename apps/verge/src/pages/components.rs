//! Value-like visual components; retained behavior belongs to feature entities.
use gpui::*;
use gpui_component::{ActiveTheme as _, Icon, IconName, StyledExt as _, v_flex};

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
        .p_4()
        .gap_4()
        .bg(cx.theme().tiles)
        .border_1()
        .border_color(cx.theme().border)
        .rounded(px(14.))
}

pub fn page_heading(
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    cx: &App,
) -> Div {
    v_flex()
        .gap_2()
        .flex_shrink_0()
        .child(div().text_2xl().font_medium().child(title.into()))
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(subtitle.into()),
        )
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
        .outline()
        .small()
        .children(MODES.map(|mode| {
            Button::new(format!("mode-{mode:?}"))
                .debug_selector(move || format!("mode-{mode:?}"))
                .label(super::home::mode_label(view.lang(), mode))
                .disabled(view.state.mode.is_none() || view.is_pending(&["mode"]))
                .selected(view.state.mode == Some(mode))
        }))
        .on_click(cx.listener(|this, clicked: &Vec<usize>, _, cx| {
            if let Some(&ix) = clicked.first() {
                this.dispatch(UiAction::SetMode(MODES[ix]), cx);
            }
        }))
}
