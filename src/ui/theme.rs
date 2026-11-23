//! Visual style: a calm dark/light palette with one accent colour.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Theme, Vec2};

use crate::config::ThemePref;

pub const ACCENTS: [(&str, [u8; 3]); 6] = [
    ("Violet", [124, 92, 255]),
    ("Blue", [59, 130, 246]),
    ("Teal", [20, 184, 166]),
    ("Green", [34, 197, 94]),
    ("Amber", [245, 158, 11]),
    ("Rose", [244, 63, 94]),
];

pub fn accent(rgb: [u8; 3]) -> Color32 {
    Color32::from_rgb(rgb[0], rgb[1], rgb[2])
}

pub fn apply(ctx: &egui::Context, pref: ThemePref, rgb: [u8; 3], zoom: f32) {
    let a = accent(rgb);
    ctx.set_visuals_of(Theme::Dark, visuals(true, a));
    ctx.set_visuals_of(Theme::Light, visuals(false, a));
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Heading, FontId::new(22.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.5, FontFamily::Monospace)),
        ]
        .into();
        s.spacing.item_spacing = Vec2::new(8.0, 7.0);
        s.spacing.button_padding = Vec2::new(10.0, 5.0);
        s.spacing.interact_size.y = 26.0;
        s.spacing.slider_width = 220.0;
        s.spacing.combo_width = 120.0;
        s.spacing.menu_margin = Margin::same(8);
        s.spacing.scroll = egui::style::ScrollStyle::solid();
        s.spacing.scroll.bar_width = 6.0;
    });
    ctx.set_theme(match pref {
        ThemePref::System => egui::ThemePreference::System,
        ThemePref::Dark => egui::ThemePreference::Dark,
        ThemePref::Light => egui::ThemePreference::Light,
    });
    ctx.set_zoom_factor(zoom);
}

fn visuals(dark: bool, accent: Color32) -> egui::Visuals {
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    let (bg, panel, card, extreme, text, weak_stroke) = if dark {
        (
            Color32::from_rgb(17, 18, 22),
            Color32::from_rgb(22, 23, 28),
            Color32::from_rgb(30, 32, 38),
            Color32::from_rgb(12, 13, 16),
            Color32::from_rgb(226, 228, 235),
            Color32::from_rgb(44, 46, 54),
        )
    } else {
        (
            Color32::from_rgb(244, 245, 248),
            Color32::from_rgb(250, 250, 252),
            Color32::from_rgb(236, 238, 243),
            Color32::from_rgb(255, 255, 255),
            Color32::from_rgb(28, 30, 36),
            Color32::from_rgb(218, 221, 228),
        )
    };
    let r = CornerRadius::same(8);
    v.panel_fill = panel;
    v.window_fill = panel;
    v.faint_bg_color = card;
    v.extreme_bg_color = extreme;
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_stroke = Stroke::new(1.0, weak_stroke);
    v.window_shadow =
        Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(if dark { 110 } else { 40 }) };
    v.popup_shadow = v.window_shadow;
    v.hyperlink_color = accent;
    v.selection.bg_fill = accent.gamma_multiply(if dark { 0.55 } else { 0.35 });
    v.selection.stroke = Stroke::new(1.0, accent);
    v.slider_trailing_fill = true;
    v.striped = false;
    v.code_bg_color = card;
    v.override_text_color = None;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = bg;
    w.noninteractive.weak_bg_fill = card;
    w.noninteractive.bg_stroke = Stroke::new(1.0, weak_stroke);
    w.noninteractive.fg_stroke = Stroke::new(1.0, text);
    w.noninteractive.corner_radius = r;

    // Controls sit on cards, so they need their own shade (slider rails, buttons).
    let control = if dark { Color32::from_rgb(47, 50, 60) } else { Color32::from_rgb(219, 222, 231) };
    let hover = if dark { Color32::from_rgb(58, 61, 73) } else { Color32::from_rgb(206, 210, 221) };
    w.inactive.bg_fill = control;
    w.inactive.weak_bg_fill = control;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, text);
    w.inactive.corner_radius = r;

    w.hovered.bg_fill = hover;
    w.hovered.weak_bg_fill = hover;
    w.hovered.bg_stroke = Stroke::new(1.0, accent.gamma_multiply(0.6));
    w.hovered.fg_stroke = Stroke::new(1.5, text);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = accent;
    w.active.weak_bg_fill = accent.gamma_multiply(0.8);
    w.active.bg_stroke = Stroke::new(1.0, accent);
    // Also egui's "strong" text colour, so it must contrast with the background.
    w.active.fg_stroke = Stroke::new(1.5, if dark { Color32::WHITE } else { Color32::from_rgb(8, 9, 12) });
    w.active.corner_radius = r;
    w.active.expansion = 0.0;

    w.open.bg_fill = hover;
    w.open.weak_bg_fill = hover;
    w.open.corner_radius = r;
    v
}

/// Background for the content "cards".
pub fn card(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(14))
}
