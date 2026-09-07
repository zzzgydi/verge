use gpui::*;
use gpui_component::{ActiveTheme as _, StyledExt as _, h_flex, v_flex};

/// A settings row owns presentation only; input state stays in retained entities.
#[derive(IntoElement, Default)]
pub(super) struct SettingRow {
    label: SharedString,
    description: Option<SharedString>,
    children: Vec<AnyElement>,
}

pub(super) fn field() -> SettingRow {
    SettingRow::default()
}

impl SettingRow {
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = label.into();
        self
    }
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }
}
impl ParentElement for SettingRow {
    fn extend(&mut self, children: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(children);
    }
}
impl RenderOnce for SettingRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        h_flex()
            .w_full()
            .gap_6()
            .py_4()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border.opacity(0.5))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(div().text_sm().font_medium().child(self.label))
                    .children(self.description.map(|text| {
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(text)
                    })),
            )
            .child(div().w(px(310.)).flex_shrink_0().children(self.children))
    }
}

pub(super) fn v_form() -> Div {
    v_flex().w_full()
}

#[derive(IntoElement, Default)]
pub(super) struct SettingsSection {
    id: Option<ElementId>,
    title: SharedString,
    children: Vec<AnyElement>,
}
impl SettingsSection {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = title.into();
        self
    }
}
impl ParentElement for SettingsSection {
    fn extend(&mut self, children: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(children);
    }
}
impl RenderOnce for SettingsSection {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        super::super::components::panel(cx)
            .id(self.id.unwrap_or("settings-section".into()))
            .gap_1()
            .px_5()
            .py_3()
            .child(div().py_2().text_lg().font_medium().child(self.title))
            .children(self.children)
    }
}
