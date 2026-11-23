//! 88‑key keyboard visualiser.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use crate::keymap::{Layout, Modifier};

const FIRST: u8 = 21;
const LAST: u8 = 108;
const WHITE_KEYS: f32 = 52.0;

fn is_black(n: u8) -> bool {
    matches!(n % 12, 1 | 3 | 6 | 8 | 10)
}

/// Draw the keyboard. `active` is a 128‑bit note mask.
pub fn piano(ui: &mut egui::Ui, height: f32, active: u128, layout: Option<&Layout>, labels: bool, accent: Color32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let p = ui.painter_at(rect);
    let dark = ui.visuals().dark_mode;
    let ww = rect.width() / WHITE_KEYS;
    let bw = ww * 0.62;
    let bh = rect.height() * 0.62;
    let on = |n: u8| active >> n & 1 == 1;
    let mapped = |n: u8| layout.is_none_or(|l| l.get(n).is_some());

    let white_fill = if dark { Color32::from_rgb(232, 233, 238) } else { Color32::WHITE };
    let white_ctrl = if dark { Color32::from_rgb(196, 200, 216) } else { Color32::from_rgb(226, 230, 242) };
    let white_dim = if dark { Color32::from_rgb(120, 122, 132) } else { Color32::from_rgb(200, 202, 210) };
    let black_fill = if dark { Color32::from_rgb(14, 14, 18) } else { Color32::from_rgb(34, 35, 42) };
    let black_dim = if dark { Color32::from_rgb(52, 53, 60) } else { Color32::from_rgb(110, 112, 122) };
    let edge = Stroke::new(1.0, if dark { Color32::from_rgb(20, 21, 26) } else { Color32::from_rgb(190, 192, 200) });
    let font = FontId::proportional((ww * 0.62).clamp(7.0, 11.0));
    let show_labels = labels && ww >= 11.0;

    let mut white_x = Vec::with_capacity(128);
    let mut wi = 0.0f32;
    for n in FIRST..=LAST {
        white_x.push(wi);
        if !is_black(n) {
            let x = rect.left() + wi * ww;
            let r = Rect::from_min_max(Pos2::new(x, rect.top()), Pos2::new(x + ww, rect.bottom()));
            let ctrl = layout.and_then(|l| l.get(n)).is_some_and(|s| s.modifier == Modifier::Ctrl);
            let fill = if on(n) {
                accent
            } else if ctrl {
                white_ctrl
            } else if mapped(n) {
                white_fill
            } else {
                white_dim
            };
            p.rect(
                r.shrink2(Vec2::new(0.5, 0.0)),
                CornerRadius { nw: 0, ne: 0, sw: 3, se: 3 },
                fill,
                edge,
                egui::StrokeKind::Inside,
            );
            if show_labels && let Some(s) = layout.and_then(|l| l.get(n)) {
                let color = if on(n) { Color32::WHITE } else { Color32::from_rgb(70, 72, 82) };
                p.text(Pos2::new(r.center().x, r.bottom() - 4.0), Align2::CENTER_BOTTOM, s.label, font.clone(), color);
            }
            if n % 12 == 0 && !show_labels {
                p.circle_filled(Pos2::new(r.center().x, r.bottom() - 6.0), 1.8, Color32::from_gray(150));
            }
            wi += 1.0;
        }
    }
    for n in FIRST..=LAST {
        if is_black(n) {
            let x = rect.left() + white_x[(n - FIRST) as usize] * ww - bw / 2.0;
            let r = Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(bw, bh));
            let fill = if on(n) {
                accent.gamma_multiply(0.85)
            } else if mapped(n) {
                black_fill
            } else {
                black_dim
            };
            p.rect_filled(r, CornerRadius { nw: 0, ne: 0, sw: 2, se: 2 }, fill);
            if show_labels && let Some(s) = layout.and_then(|l| l.get(n)) {
                p.text(
                    Pos2::new(r.center().x, r.bottom() - 3.0),
                    Align2::CENTER_BOTTOM,
                    s.label,
                    FontId::proportional((bw * 0.8).clamp(6.5, 10.0)),
                    Color32::from_gray(225),
                );
            }
        }
    }
}
