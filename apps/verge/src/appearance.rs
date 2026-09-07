//! Application palette. Component behavior continues to come from gpui-component.
use gpui::{App, Hsla, px, rgb};
use gpui_component::theme::{Theme, ThemeMode};

pub fn apply(mode: ThemeMode, cx: &mut App) {
    let dark = mode == ThemeMode::Dark;
    let color = |dark_value, light_value| -> Hsla {
        rgb(if dark { dark_value } else { light_value }).into()
    };
    let theme = Theme::global_mut(cx);
    theme.font_size = px(14.);
    theme.radius = px(8.);
    theme.radius_lg = px(14.);
    let c = &mut theme.colors;
    c.background = color(0x141416, 0xf4f4f5);
    c.foreground = color(0xf0f0f2, 0x222226);
    c.border = color(0x303035, 0xe0e0e4);
    c.muted = color(0x2a2a2f, 0xe9e9ed);
    c.muted_foreground = color(0x99999f, 0x686870);
    c.tiles = color(0x1e1e21, 0xffffff);
    c.group_box = c.tiles;
    c.group_box_foreground = c.foreground;
    c.title_bar = c.background;
    c.title_bar_border = c.background;
    c.sidebar = c.background;
    c.sidebar_border = c.background;
    c.sidebar_foreground = c.muted_foreground;
    c.sidebar_accent = color(0x27272d, 0xe4e4e9);
    c.sidebar_accent_foreground = c.foreground;
    c.sidebar_primary = c.foreground;
    c.sidebar_primary_foreground = c.background;
    c.accent = c.sidebar_accent;
    c.accent_foreground = c.foreground;
    c.primary = color(0xf3f3f5, 0x252529);
    c.primary_foreground = color(0x19191c, 0xffffff);
    c.primary_hover = color(0xdcdce1, 0x414149);
    c.primary_active = color(0xc8c8ce, 0x111114);
    c.button_primary = c.primary;
    c.button_primary_foreground = c.primary_foreground;
    c.button_primary_hover = c.primary_hover;
    c.button_primary_active = c.primary_active;
    c.button = c.tiles;
    c.button_foreground = c.foreground;
    c.button_hover = c.accent;
    c.button_active = c.muted;
    c.secondary = c.accent;
    c.secondary_foreground = c.foreground;
    c.input = c.border;
    c.list = c.tiles;
    c.list_active = c.accent;
    c.list_active_border = c.border;
    c.list_hover = c.accent;
    c.table = c.tiles;
    c.table_head = c.tiles;
    c.table_even = color(0x222226, 0xf8f8fa);
    c.table_hover = c.accent;
    c.table_row_border = c.border;
    c.popover = c.tiles;
    c.popover_foreground = c.foreground;
    c.ring = color(0xa6a6b6, 0x707080);
    c.chart_1 = c.foreground;
    c.chart_2 = c.muted_foreground;
    theme.tokens = theme.colors.into();
    Theme::sync_base(cx);
}
