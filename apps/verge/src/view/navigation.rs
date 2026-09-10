use super::*;

// Critically damped: responsive without overshoot. Stable IDs preserve position
// and velocity when a second click reverses an unfinished transition.
fn sidebar_motion(target: f32) -> SpringAnimation<f32> {
    SpringAnimation::new(SpringConfig::new(625., 50., 1.)).to(target)
}

fn reveal(element: Div, id: impl Into<ElementId>, expanded: f32) -> impl IntoElement {
    element.with_spring(id, sidebar_motion(expanded), |element, value| {
        element.opacity(value.clamp(0., 1.))
    })
}

impl MainView {
    pub(crate) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
        cx.notify();
    }

    pub(super) fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let collapsed = self.sidebar_collapsed;
        let expanded = if collapsed { 0. } else { 1. };
        let lang = self.lang();
        let mut navigation = v_flex().gap_6();
        for (label, entries) in [
            (
                "nav.group.proxy",
                vec![
                    (Page::Home, "home.title", IconName::LayoutDashboard),
                    (Page::Proxies, "proxies.title", IconName::Globe),
                    (Page::Rules, "rules.title", IconName::BookOpen),
                    (Page::Connections, "connections.title", IconName::Network),
                ],
            ),
            (
                "nav.group.system",
                vec![
                    (Page::Profiles, "profiles.title", IconName::File),
                    (Page::Logs, "logs.title", IconName::SquareTerminal),
                    (Page::Settings, "settings.title", IconName::Settings),
                ],
            ),
        ] {
            navigation = navigation.child(
                v_flex()
                    .gap_1()
                    .child(reveal(
                        div()
                            .h_6()
                            .px_3()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(px(12.))
                            .text_color(cx.theme().muted_foreground)
                            .child(tr(lang, label)),
                        SharedString::from(format!("sidebar-heading-{label}")),
                        expanded,
                    ))
                    .children(entries.into_iter().map(|(page, label, icon)| {
                        Button::new(format!("nav-{page:?}"))
                            .debug_selector(move || format!("nav-{page:?}"))
                            .ghost()
                            .w_full()
                            .h(px(40.))
                            .px(px(15.))
                            .rounded_lg()
                            .border_1()
                            .border_color(gpui::transparent_black())
                            .accessibility_label(tr(lang, label))
                            .child(
                                h_flex()
                                    .w_full()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .child(Icon::new(icon).size(px(18.)).flex_shrink_0())
                                    .child(reveal(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_size(px(14.))
                                            .child(tr(lang, label)),
                                        SharedString::from(format!("sidebar-label-{page:?}")),
                                        expanded,
                                    ))
                                    .with_spring(
                                        SharedString::from(format!("sidebar-label-gap-{page:?}")),
                                        sidebar_motion(12. * expanded),
                                        |row, gap| row.gap(px(gap.max(0.))),
                                    ),
                            )
                            .when(self.state.page == page, |button| {
                                button
                                    .bg(cx.theme().list_active)
                                    .border_color(cx.theme().list_active_border)
                            })
                            .tooltip(tr(lang, label))
                            .on_click(cx.listener(move |this, _, _, cx| this.navigate(page, cx)))
                    })),
            );
        }
        v_flex()
            .id("verge-sidebar")
            .debug_selector(|| "verge-sidebar".into())
            .h_full()
            .flex_shrink_0()
            .px(px(4.))
            .overflow_hidden()
            .gap_4()
            .child(
                h_flex()
                    .h_16()
                    .w_full()
                    .flex_shrink_0()
                    .px(px(8.))
                    .child(reveal(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .gap_2()
                            .whitespace_nowrap()
                            .child(
                                svg()
                                    .path("branding/logo.svg")
                                    .size_7()
                                    .flex_shrink_0()
                                    .text_color(cx.theme().foreground),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_xl()
                                    .font_semibold()
                                    .child("Verge"),
                            ),
                        "sidebar-brand",
                        expanded,
                    ))
                    .child(
                        Button::new("sidebar-toggle")
                            .debug_selector(|| "sidebar-toggle".into())
                            .ghost()
                            .small()
                            .size(px(32.))
                            .icon(if collapsed {
                                IconName::PanelLeftOpen
                            } else {
                                IconName::PanelLeftClose
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_sidebar(cx);
                            })),
                    ),
            )
            .child(navigation)
            .child(div().flex_1())
            .with_spring(
                "sidebar-width",
                sidebar_motion(56. + 148. * expanded),
                |sidebar, width| sidebar.w(px(width.clamp(56., 204.))),
            )
    }
}
