//! Application palette. Component behavior continues to come from GPUI Kit.
pub mod metrics;
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{App, Hsla, px, rgb};

// Sampled from the upper purple face of assets/icons/icon.icns.
const BRAND_PURPLE: u32 = 0x761e8d;

pub fn apply(mode: ThemeMode, cx: &mut App) {
    let dark = mode == ThemeMode::Dark;
    let color = |dark_value, light_value| -> Hsla {
        rgb(if dark { dark_value } else { light_value }).into()
    };
    let theme = Theme::global_mut(cx);
    theme.font_size = px(metrics::REM);
    theme.radius = px(8.);
    theme.radius_lg = px(metrics::PANEL_RADIUS);
    let c = &mut theme.colors;
    c.background = color(0x191620, 0xf7f4fa);
    c.foreground = color(0xf4eff7, 0x2a2333);
    c.border = color(0x3a3245, 0xe6deeb);
    c.muted = color(0x302938, 0xede6f3);
    c.muted_foreground = color(0xb1a7bd, 0x6c6075);
    c.tiles = color(0x221e2b, 0xffffff);
    c.group_box = c.tiles;
    c.group_box_foreground = c.foreground;
    c.title_bar = c.background;
    c.title_bar_border = c.background;
    c.status_bar = c.background;
    c.status_bar_border = c.border;
    c.sidebar = c.background;
    c.sidebar_border = c.background;
    c.sidebar_foreground = c.muted_foreground;
    c.sidebar_accent = color(0x33233f, 0xf2e8f7);
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
    c.button = c.tiles;
    c.button_foreground = c.foreground;
    c.button_hover = c.accent;
    c.button_active = c.muted;
    c.secondary = c.accent;
    c.secondary_foreground = c.accent_foreground;
    c.secondary_hover = c.muted;
    c.secondary_active = c.muted;
    c.button_secondary = c.secondary;
    c.button_secondary_foreground = c.secondary_foreground;
    c.button_secondary_hover = c.secondary_hover;
    c.button_secondary_active = c.secondary_active;
    c.input = c.border;
    c.list = c.tiles;
    c.list_active = color(0x392744, 0xf0e3f6);
    c.list_active_border = color(0xbb83d0, 0xaf77c0);
    c.list_hover = c.accent;
    c.table = c.tiles;
    c.table_head = c.tiles;
    c.table_even = color(0x272230, 0xfaf7fc);
    c.table_active = c.list_active;
    c.table_active_border = c.list_active_border;
    c.table_hover = c.accent;
    c.table_row_border = c.border;
    c.popover = c.tiles;
    c.popover_foreground = c.foreground;
    c.ring = c.primary;
    c.caret = c.primary;
    c.link = c.primary;
    c.link_hover = c.primary_hover;
    c.link_active = c.primary_active;
    c.progress_bar = c.primary;
    c.slider_bar = c.primary;
    c.switch = c.muted;
    c.selection = c.primary.opacity(0.22);
    c.tab_active = c.accent;
    c.tab_active_foreground = c.accent_foreground;
    // Semantic text must remain legible on neutral and tinted surfaces.
    c.success = color(0x4ade80, 0x126b35);
    c.warning = color(0xfacc15, 0x805000);
    c.danger = color(0xfda4af, 0xb42332);
    c.chart_1 = c.primary;
    c.chart_2 = color(0x51cfc3, 0x0c7066);
    theme.tokens = theme.colors.into();
    Theme::sync_base(cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::ActiveTheme as _;
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
                let neutral = if mode == ThemeMode::Dark {
                    rgb(0xbabac3).into()
                } else {
                    rgb(0x50505a).into()
                };
                let opacities = if mode == ThemeMode::Dark {
                    [0., 0.09, 0.13, 0.16]
                } else {
                    [0., 0.04, 0.06, 0.08]
                };
                for fg in [t.success, t.warning, t.danger, neutral] {
                    for surface in [t.tiles, t.background, t.list_active, t.list_hover] {
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
                    for bg in [t.tiles, t.background, t.list_active, t.list_hover] {
                        assert!(contrast(fg, bg) >= 4.5, "{mode:?}: {fg:?} on {bg:?}");
                    }
                }
                for bg in [t.primary, t.primary_hover, t.primary_active] {
                    assert!(contrast(t.primary_foreground, bg) >= 4.5);
                }
                assert!((t.chart_1.h - t.chart_2.h).abs() > 0.2);
            }
        });
    }
}
