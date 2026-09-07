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
