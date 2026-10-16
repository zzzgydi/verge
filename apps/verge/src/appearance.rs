//! Application palette. Component behavior continues to come from GPUI Kit.
pub mod metrics;
use gpui_kit::component::{
    ActiveTheme as _,
    theme::{Theme, ThemeMode},
};
use gpui_kit::{App, Hsla, px, rgb};

// Sampled from the upper purple face of assets/icons/icon.icns.
const BRAND_PURPLE: u32 = 0x761e8d;

pub fn delay_neutral(cx: &App) -> Hsla {
    rgb(if cx.theme().is_dark() {
        0xbabac3
    } else {
        0x50505a
    })
    .into()
}

pub fn delay_color(delay: u32, cx: &App) -> Hsla {
    match delay {
        0 => cx.theme().danger,
        1..200 => cx.theme().success,
        200..800 => cx.theme().warning,
        _ => delay_neutral(cx),
    }
}

pub fn apply(mode: ThemeMode, cx: &mut App) {
    let dark = mode == ThemeMode::Dark;
    let color = |dark_value, light_value| -> Hsla {
        rgb(if dark { dark_value } else { light_value }).into()
    };
    let theme = Theme::global_mut(cx);
    theme.font_size = px(metrics::REM);
    theme.radius = px(6.);
    theme.radius_lg = px(metrics::PANEL_RADIUS);
    let c = &mut theme.colors;
    c.background = color(0x202022, 0xf8f8f9);
    c.foreground = color(0xf0f0f2, 0x242426);
    c.border = color(0x3c3c40, 0xe1e1e5);
    c.muted = color(0x323235, 0xeeeef0);
    c.muted_foreground = color(0xb6b6be, 0x62626b);
    c.group_box = color(0x262628, 0xffffff);
    c.group_box_foreground = c.foreground;
    c.title_bar = color(0x28282b, 0xf0f0f2);
    c.title_bar_border = c.title_bar;
    c.status_bar = c.background;
    c.status_bar_border = c.border;
    c.sidebar = c.title_bar;
    c.sidebar_border = c.border;
    c.sidebar_foreground = c.muted_foreground;
    c.sidebar_accent = color(0x353039, 0xebe6ef);
    c.sidebar_accent_foreground = color(0xe6b8f4, BRAND_PURPLE);
    c.accent = c.sidebar_accent;
    c.accent_foreground = c.sidebar_accent_foreground;
    c.primary = color(0xd8a0ed, BRAND_PURPLE);
    c.primary_foreground = color(0x291332, 0xffffff);
    c.primary_hover = color(0xe3b7f3, 0x8a2ca2);
    c.primary_active = color(0xc784df, 0x601575);
    c.sidebar_primary = c.primary;
    c.sidebar_primary_foreground = c.primary_foreground;
    c.button_primary = c.primary;
    c.button_primary_foreground = c.primary_foreground;
    c.button_primary_hover = c.primary_hover;
    c.button_primary_active = c.primary_active;
    c.button = c.group_box;
    c.button_foreground = c.foreground;
    c.button_hover = c.muted;
    c.button_active = c.muted;
    c.secondary = c.muted;
    c.secondary_foreground = c.foreground;
    c.secondary_hover = c.muted;
    c.secondary_active = c.muted;
    c.button_secondary = c.secondary;
    c.button_secondary_foreground = c.secondary_foreground;
    c.button_secondary_hover = c.secondary_hover;
    c.button_secondary_active = c.secondary_active;
    c.input = c.border;
    c.list = c.group_box;
    c.list_active = color(0x38303e, 0xeee8f3);
    c.list_active_border = color(0x8a6a99, 0xb59cc2);
    c.list_hover = color(0x303033, 0xf1f1f3);
    c.table = c.group_box;
    c.table_head = c.group_box;
    c.table_even = color(0x2a2a2d, 0xf7f7f9);
    c.table_active = c.list_active;
    c.table_active_border = c.list_active_border;
    c.table_hover = c.list_hover;
    c.table_row_border = c.border;
    c.popover = c.group_box;
    c.popover_foreground = c.foreground;
    c.ring = c.primary;
    c.caret = c.primary;
    c.link = c.primary;
    c.link_hover = c.primary_hover;
    c.link_active = c.primary_active;
    c.progress_bar = c.primary;
    c.slider_bar = c.primary;
    c.switch = color(0x626269, 0xd4d4d9);
    c.switch_thumb = rgb(0xffffff).into();
    c.selection = c.primary.opacity(0.22);
    c.tab_active = c.accent;
    c.tab_active_foreground = c.accent_foreground;
    // Semantic text must remain legible on neutral and tinted surfaces.
    c.success = color(0x4ade80, 0x126b35);
    c.warning = color(0xfacc15, 0x805000);
    c.danger = color(0xfda4af, 0xb42332);
    c.chart_1 = c.primary;
    c.chart_2 = color(0x83baff, 0x0064c2);
    theme.tokens = theme.colors.into();
    Theme::sync_base(cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{Rgba, TestAppContext};

    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let luminance = |color: Hsla| {
            let c: Rgba = color.into();
            let linear = |v: f32| {
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
        };
        let a = luminance(a);
        let b = luminance(b);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[gpui_kit::test]
    fn semantic_text_has_aa_contrast_in_delay_states(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            for mode in [ThemeMode::Light, ThemeMode::Dark] {
                Theme::change(mode, None, cx);
                apply(mode, cx);
                let t = cx.theme();
                let neutral = delay_neutral(cx);
                let opacities = if mode == ThemeMode::Dark {
                    [0., 0.09, 0.13, 0.16]
                } else {
                    [0., 0.04, 0.06, 0.08]
                };
                for fg in [t.success, t.warning, t.danger, neutral] {
                    for surface in [t.group_box, t.background, t.list_active, t.list_hover] {
                        for alpha in opacities {
                            let background =
                                Rgba::from(surface).blend(Rgba::from(fg.opacity(alpha)));
                            let ratio = contrast(fg, background.into());
                            assert!(ratio >= 4.5, "{mode:?}: {fg:?} contrast {ratio}");
                        }
                    }
                }
            }
        });
    }

    #[gpui_kit::test]
    fn brand_and_traffic_colors_stay_readable_in_both_themes(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            for mode in [ThemeMode::Light, ThemeMode::Dark] {
                Theme::change(mode, None, cx);
                apply(mode, cx);
                let t = cx.theme();
                for fg in [t.foreground, t.muted_foreground, t.primary, t.chart_2] {
                    for bg in [t.group_box, t.background, t.list_active, t.list_hover] {
                        assert!(contrast(fg, bg) >= 4.5, "{mode:?}: {fg:?} on {bg:?}");
                    }
                }
                for bg in [t.primary, t.primary_hover, t.primary_active] {
                    assert!(contrast(t.primary_foreground, bg) >= 4.5);
                }
                // Purple and blue remain at least 43 degrees apart.
                assert!((t.chart_1.h - t.chart_2.h).abs() > 0.12);
            }
        });
    }
}
