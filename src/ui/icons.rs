//! Small vector icons painted with egui shapes (no icon font needed).

use std::f32::consts::{PI, TAU};

use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Response, Sense, Shape, Stroke, Vec2, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Play,
    Pause,
    Stop,
    Loop,
    Plus,
    Refresh,
    Search,
    Gear,
    Info,
    Piano,
    Drum,
    Midi,
    Note,
    Folder,
    Close,
    Warning,
}

fn arc(c: Pos2, r: f32, from: f32, to: f32, n: usize) -> Vec<Pos2> {
    (0..=n).map(|i| from + (to - from) * i as f32 / n as f32).map(|a| c + r * vec2(a.cos(), a.sin())).collect()
}

fn arrow_head(p: &egui::Painter, tip: Pos2, dir: Vec2, size: f32, color: Color32) {
    let d = dir.normalized();
    let n = vec2(-d.y, d.x);
    p.add(Shape::convex_polygon(
        vec![tip + d * size * 0.6, tip - d * size * 0.5 + n * size * 0.6, tip - d * size * 0.5 - n * size * 0.6],
        color,
        Stroke::NONE,
    ));
}

/// Paint `icon` centred in `rect`.
pub fn paint(p: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let w = (s * 0.11).max(1.3);
    let st = Stroke::new(w, color);
    match icon {
        Icon::Play => {
            let h = s * 0.36;
            p.add(Shape::convex_polygon(
                vec![c + vec2(-h * 0.75, -h), c + vec2(h * 1.0, 0.0), c + vec2(-h * 0.75, h)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Pause => {
            let (bw, bh) = (s * 0.17, s * 0.62);
            for dx in [-s * 0.15, s * 0.15] {
                p.rect_filled(Rect::from_center_size(c + vec2(dx, 0.0), vec2(bw, bh)), 1.5, color);
            }
        }
        Icon::Stop => {
            p.rect_filled(Rect::from_center_size(c, Vec2::splat(s * 0.52)), 2.0, color);
        }
        Icon::Loop => {
            let r = Rect::from_center_size(c, vec2(s * 0.72, s * 0.46));
            let rad = r.height() / 2.0;
            let mut pts = arc(pos2(r.right() - rad, r.center().y), rad, -PI / 2.0, PI / 2.0, 10);
            pts.extend(arc(pos2(r.left() + rad, r.center().y), rad, PI / 2.0, 1.5 * PI, 10));
            pts.push(pos2(r.center().x + s * 0.02, r.top()));
            p.add(Shape::line(pts, st));
            arrow_head(p, pos2(r.center().x + s * 0.08, r.top()), vec2(1.0, 0.0), s * 0.26, color);
        }
        Icon::Plus => {
            let h = s * 0.32;
            p.line_segment([c - vec2(h, 0.0), c + vec2(h, 0.0)], st);
            p.line_segment([c - vec2(0.0, h), c + vec2(0.0, h)], st);
        }
        Icon::Refresh => {
            let r = s * 0.3;
            p.add(Shape::line(arc(c, r, -PI * 0.35, PI * 1.45, 24), st));
            let a = -PI * 0.35;
            arrow_head(p, c + r * vec2(a.cos(), a.sin()), vec2(-a.sin(), a.cos()), s * 0.3, color);
        }
        Icon::Search => {
            let r = s * 0.24;
            let cc = c - vec2(s * 0.08, s * 0.08);
            p.circle_stroke(cc, r, st);
            let d = vec2(1.0, 1.0).normalized();
            p.line_segment([cc + d * r, cc + d * (r + s * 0.24)], Stroke::new(w * 1.3, color));
        }
        Icon::Gear => {
            let r = s * 0.27;
            for i in 0..8 {
                let a = i as f32 * TAU / 8.0;
                let d = vec2(a.cos(), a.sin());
                p.line_segment([c + d * r * 0.8, c + d * (r + s * 0.12)], Stroke::new(s * 0.14, color));
            }
            p.circle_stroke(c, r, Stroke::new(w * 1.2, color));
            p.circle_stroke(c, s * 0.09, st);
        }
        Icon::Info => {
            p.circle_stroke(c, s * 0.36, st);
            p.circle_filled(c - vec2(0.0, s * 0.15), w * 0.8, color);
            p.line_segment([c - vec2(0.0, s * 0.03), c + vec2(0.0, s * 0.2)], st);
        }
        Icon::Piano => {
            let r = Rect::from_center_size(c, vec2(s * 0.8, s * 0.6));
            p.rect_stroke(r, 2.0, st, egui::StrokeKind::Middle);
            for i in 1..4 {
                let x = r.left() + r.width() * i as f32 / 4.0;
                p.line_segment([pos2(x, r.top() + r.height() * 0.5), pos2(x, r.bottom())], Stroke::new(w * 0.7, color));
            }
            for i in [1, 3] {
                let x = r.left() + r.width() * i as f32 / 4.0;
                p.rect_filled(
                    Rect::from_center_size(pos2(x, r.top() + r.height() * 0.28), vec2(s * 0.12, r.height() * 0.5)),
                    0.0,
                    color,
                );
            }
        }
        Icon::Drum => {
            let (rx, ry) = (s * 0.33, s * 0.11);
            let top = c.y - s * 0.08;
            let ell = |cy: f32, from: f32, to: f32| -> Vec<Pos2> {
                (0..=20)
                    .map(|i| from + (to - from) * i as f32 / 20.0)
                    .map(|a| pos2(c.x + rx * a.cos(), cy + ry * a.sin()))
                    .collect()
            };
            p.add(Shape::closed_line(ell(top, 0.0, TAU), st));
            p.add(Shape::line(ell(top + s * 0.3, 0.0, PI), st));
            p.line_segment([pos2(c.x - rx, top), pos2(c.x - rx, top + s * 0.3)], st);
            p.line_segment([pos2(c.x + rx, top), pos2(c.x + rx, top + s * 0.3)], st);
            p.line_segment([c + vec2(-s * 0.05, -s * 0.2), c + vec2(-s * 0.32, -s * 0.42)], st);
            p.line_segment([c + vec2(s * 0.05, -s * 0.2), c + vec2(s * 0.32, -s * 0.42)], st);
        }
        Icon::Midi => {
            p.circle_stroke(c, s * 0.36, st);
            for i in 0..5 {
                let a = PI + i as f32 * PI / 4.0;
                p.circle_filled(c + s * 0.2 * vec2(a.cos(), a.sin()) + vec2(0.0, s * 0.04), w * 0.75, color);
            }
        }
        Icon::Note => {
            let head = c + vec2(-s * 0.12, s * 0.22);
            p.add(Shape::ellipse_filled(head, vec2(s * 0.16, s * 0.12), color));
            let top = pos2(head.x + s * 0.14, c.y - s * 0.36);
            p.line_segment([pos2(top.x, head.y), top], Stroke::new(w * 1.1, color));
            p.add(Shape::line(
                vec![top, top + vec2(s * 0.2, s * 0.12), top + vec2(s * 0.18, s * 0.3)],
                Stroke::new(w * 1.1, color),
            ));
        }
        Icon::Folder => {
            let r = Rect::from_center_size(c + vec2(0.0, s * 0.03), vec2(s * 0.76, s * 0.54));
            p.add(Shape::closed_line(
                vec![
                    r.left_top(),
                    pos2(r.left() + s * 0.26, r.top()),
                    pos2(r.left() + s * 0.34, r.top() + s * 0.09),
                    pos2(r.right(), r.top() + s * 0.09),
                    r.right_bottom(),
                    r.left_bottom(),
                ],
                st,
            ));
        }
        Icon::Close => {
            let h = s * 0.24;
            p.line_segment([c - vec2(h, h), c + vec2(h, h)], st);
            p.line_segment([c + vec2(-h, h), c + vec2(h, -h)], st);
        }
        Icon::Warning => {
            let h = s * 0.4;
            p.add(Shape::convex_polygon(
                vec![c + vec2(0.0, -h), c + vec2(h * 1.1, h * 0.85), c + vec2(-h * 1.1, h * 0.85)],
                color,
                Stroke::NONE,
            ));
            let dark = Color32::from_rgb(30, 30, 30);
            p.line_segment([c + vec2(0.0, -h * 0.35), c + vec2(0.0, h * 0.3)], Stroke::new(w, dark));
            p.circle_filled(c + vec2(0.0, h * 0.58), w * 0.6, dark);
        }
    }
}

/// Look of an [`icon_button`].
#[derive(Clone, Copy, Default)]
pub struct Style {
    /// Solid background (primary buttons). `None` = normal widget look.
    pub fill: Option<Color32>,
    /// Draw as selected (accent tint).
    pub selected: bool,
    pub min_size: Vec2,
    pub text_size: Option<f32>,
    pub strong: bool,
    /// No background unless hovered.
    pub frameless: bool,
    /// Left‑align icon and label instead of centring them.
    pub align_left: bool,
}

/// Button with a painted icon and optional label.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, label: &str, style: Style) -> Response {
    let font = egui::FontId::proportional(style.text_size.unwrap_or(14.0));
    let pad = vec2(10.0, 5.0);
    let icon_size = (font.size * 1.15).round();
    let galley =
        (!label.is_empty()).then(|| ui.painter().layout_no_wrap(label.to_owned(), font.clone(), Color32::WHITE));
    let text_w = galley.as_ref().map_or(0.0, |g| g.size().x + 7.0);
    let desired = vec2(pad.x * 2.0 + icon_size + text_w, (icon_size + pad.y * 2.0).max(ui.spacing().interact_size.y))
        .max(style.min_size);
    let (rect, resp) = ui.allocate_exact_size(desired, Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let enabled = ui.is_enabled();
    let v = ui.visuals();
    let wv = v.widgets.style(&resp);
    let accent = v.selection.stroke.color;
    let (bg, fg) = if let Some(fill) = style.fill {
        let f = if resp.hovered() && enabled { fill.gamma_multiply(1.12) } else { fill };
        (f, Color32::WHITE)
    } else if style.selected {
        (accent.gamma_multiply(if v.dark_mode { 0.35 } else { 0.18 }), accent)
    } else if style.frameless && !resp.hovered() {
        (Color32::TRANSPARENT, wv.fg_stroke.color)
    } else {
        (wv.weak_bg_fill, wv.fg_stroke.color)
    };
    let fg = if enabled { fg } else { fg.gamma_multiply(0.45) };
    let bg = if enabled { bg } else { bg.gamma_multiply(0.6) };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(8), bg);
    let content_w = icon_size + text_w;
    let x0 = if style.align_left { rect.left() + pad.x + 2.0 } else { rect.center().x - content_w / 2.0 };
    let icon_rect = Rect::from_min_size(pos2(x0, rect.center().y - icon_size / 2.0), Vec2::splat(icon_size));
    paint(p, icon_rect, icon, fg);
    if let Some(g) = galley {
        let pos = pos2(icon_rect.right() + 7.0, rect.center().y - g.size().y / 2.0);
        p.galley_with_override_text_color(pos, g.clone(), fg);
        if style.strong {
            p.galley_with_override_text_color(pos + vec2(0.4, 0.0), g, fg);
        }
    }
    resp
}

/// Small coloured warning line.
pub fn warn_label(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
        paint(ui.painter(), r, Icon::Warning, ui.visuals().warn_fg_color);
        ui.label(egui::RichText::new(text).small().color(ui.visuals().warn_fg_color));
    });
}
