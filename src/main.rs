#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod engine;
mod hotkeys;
mod keymap;
mod live;
mod mapper;
mod output;
mod song;
mod ui;

use eframe::egui;

/// 64×64 app icon drawn in code (no image files, no image decoder).
fn icon() -> egui::IconData {
    const S: usize = 64;
    let mut rgba = vec![0u8; S * S * 4];
    for y in 0..S {
        for x in 0..S {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Rounded square mask.
            let r = 14.0;
            let dx = (fx - fx.clamp(r, S as f32 - r)).abs();
            let dy = (fy - fy.clamp(r, S as f32 - r)).abs();
            let d = (dx * dx + dy * dy).sqrt();
            let alpha = (r + 0.5 - d).clamp(0.0, 1.0);
            if alpha <= 0.0 {
                continue;
            }
            let t = fy / S as f32;
            let mut c = [(124.0 - 30.0 * t) as u8, (92.0 - 20.0 * t) as u8, 255, (alpha * 255.0) as u8];
            // Three white keys with two black keys.
            let (kx0, kx1, ky0, ky1) = (12.0, 52.0, 18.0, 48.0);
            if (kx0..kx1).contains(&fx) && (ky0..ky1).contains(&fy) {
                let w = (kx1 - kx0) / 3.0;
                let in_gap = ((fx - kx0) % w) < 1.5 && fx > kx0 + 1.0;
                let black = fy < 36.0 && [(kx0 + w), (kx0 + 2.0 * w)].iter().any(|bx| (fx - bx).abs() < 4.5);
                c = if black {
                    [24, 24, 32, c[3]]
                } else if in_gap {
                    [200, 200, 215, c[3]]
                } else {
                    [255, 255, 255, c[3]]
                };
            }
            rgba[(y * S + x) * 4..][..4].copy_from_slice(&c);
        }
    }
    egui::IconData { rgba, width: S as u32, height: S as u32 }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("nanoMIDI")
            .with_app_id("nanomidi")
            .with_inner_size([1000.0, 700.0])
            .with_min_inner_size([760.0, 520.0])
            .with_drag_and_drop(true)
            .with_icon(std::sync::Arc::new(icon())),
        ..Default::default()
    };
    eframe::run_native("nanoMIDI", options, Box::new(|cc| Ok(Box::new(ui::App::new(cc)))))
}
