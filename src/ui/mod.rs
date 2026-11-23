//! The egui front end.

mod icons;
mod piano;
mod theme;

use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, Layout as ELayout, RichText, Sense, Vec2};

use icons::{Icon, Style, icon_button, warn_label};

use crate::config::{Config, KeyOptions, LiveConfig, OutputMode, PlayerConfig, ThemePref};
use crate::engine::{Cmd, EngineConfig, EngineHandle, Log, MAX_SPEED, MIN_SPEED, PlayState};
use crate::hotkeys::{Action, Hotkeys, hotkey_string};
use crate::keymap::{self, DrumPad, KeyMapConfig, Layout, LayoutKind, NOTE_NAMES, note_name};
use crate::live::LiveInput;
use crate::output::{midi_input_ports, midi_output_ports};
use crate::song::{Song, fmt_time};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Player,
    Live,
    Settings,
    About,
}

struct LibEntry {
    path: PathBuf,
    name: String,
    lower: String,
}

impl LibEntry {
    fn new(path: PathBuf) -> Self {
        let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let lower = name.to_lowercase();
        Self { path, name, lower }
    }
}

type Notify = Arc<dyn Fn() + Send + Sync>;

pub struct App {
    cfg: Config,
    cfg_path: PathBuf,
    saved_cfg: Config,
    last_save_check: Instant,

    engine: EngineHandle,
    engine_snapshot: (PlayerConfig, KeyMapConfig, Vec<DrumPad>),
    hotkeys: Hotkeys,
    applied_hotkeys: Option<crate::config::Hotkeys>,
    hotkey_problems: Vec<String>,
    rebinding: Option<Action>,

    page: Page,
    library: Vec<LibEntry>,
    scan_rx: Option<mpsc::Receiver<Vec<PathBuf>>>,
    search: String,
    song: Option<Arc<Song>>,
    loading: Option<(PathBuf, mpsc::Receiver<Result<Song, String>>)>,
    muted: Vec<bool>,
    seek_drag: Option<f64>,

    midi_outs: Vec<String>,
    midi_ins: Vec<String>,
    live: Option<LiveInput>,
    live_log: Arc<Log>,
    live_snapshot: (LiveConfig, KeyMapConfig, Vec<DrumPad>),

    notify: Notify,
    applied_theme: Option<(ThemePref, [u8; 3], f32)>,
    applied_on_top: Option<bool>,
    toast: Option<(String, Instant)>,
}

fn engine_config(cfg: &Config) -> EngineConfig {
    let p = &cfg.player;
    EngineConfig {
        mode: p.mode,
        midi_port: p.midi_out_port.clone(),
        piano: Layout::piano(&cfg.keymap, p.keys.keys88),
        drums: Layout::drums(&cfg.drums),
        keys: p.keys.clone(),
        humanize: p.humanize.clone(),
        loop_song: p.loop_song,
        release_on_pause: p.release_on_pause,
        speed_step: p.speed_step,
    }
}

fn live_layout(cfg: &Config) -> Layout {
    match cfg.live.layout {
        LayoutKind::Piano => Layout::piano(&cfg.keymap, cfg.live.keys.keys88),
        LayoutKind::Drums => Layout::drums(&cfg.drums),
    }
}

fn is_midi(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("mid") || e.eq_ignore_ascii_case("midi"))
}

fn scan_folders(folders: Vec<PathBuf>) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: u8, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                if depth < 4 {
                    walk(&p, depth + 1, out);
                }
            } else if is_midi(&p) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for f in folders {
        walk(&f, 0, &mut out);
    }
    out
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let cfg_path = Config::path();
        let (cfg, warning) = Config::load(&cfg_path);
        let ctx = cc.egui_ctx.clone();
        let notify: Notify = Arc::new(move || ctx.request_repaint());

        let n2 = notify.clone();
        let engine = EngineHandle::spawn(engine_config(&cfg), cfg.player.speed, Box::new(move || n2()));
        if let Some(w) = warning {
            engine.shared.log.warn(w);
        }
        let (hotkeys, hk_err) = Hotkeys::new(engine.tx.clone(), notify.clone());
        if let Some(e) = hk_err {
            engine.shared.log.warn(e);
        }

        let mut app = Self {
            saved_cfg: cfg.clone(),
            engine_snapshot: (cfg.player.clone(), cfg.keymap.clone(), cfg.drums.clone()),
            live_snapshot: (cfg.live.clone(), cfg.keymap.clone(), cfg.drums.clone()),
            library: cfg.library.iter().cloned().map(LibEntry::new).collect(),
            cfg,
            cfg_path,
            last_save_check: Instant::now(),
            engine,
            hotkeys,
            applied_hotkeys: None,
            hotkey_problems: Vec::new(),
            rebinding: None,
            page: Page::Player,
            scan_rx: None,
            search: String::new(),
            song: None,
            loading: None,
            muted: Vec::new(),
            seek_drag: None,
            midi_outs: Vec::new(),
            midi_ins: Vec::new(),
            live: None,
            live_log: Arc::new(Log::default()),
            notify,
            applied_theme: None,
            applied_on_top: None,
            toast: None,
        };
        app.rescan();
        if let Some(p) = app.cfg.last_file.clone().filter(|p| p.is_file()) {
            app.load(p);
        }
        if app.cfg.player.mode == OutputMode::MidiOut {
            app.refresh_outputs();
        }
        app
    }

    fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    fn rescan(&mut self) {
        let folders = self.cfg.folders.clone();
        let (tx, rx) = mpsc::channel();
        let notify = self.notify.clone();
        std::thread::spawn(move || {
            let _ = tx.send(scan_folders(folders));
            notify();
        });
        self.scan_rx = Some(rx);
    }

    fn refresh_outputs(&mut self) {
        match midi_output_ports() {
            Ok(p) => self.midi_outs = p,
            Err(e) => self.engine.shared.log.warn(format!("MIDI outputs: {e}")),
        }
    }

    fn refresh_inputs(&mut self) {
        match midi_input_ports() {
            Ok(p) => self.midi_ins = p,
            Err(e) => self.live_log.warn(format!("MIDI inputs: {e}")),
        }
    }

    fn add_files(&mut self, files: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
        let mut first = None;
        for f in files.into_iter().filter(|f| is_midi(f)) {
            if !self.cfg.library.contains(&f) {
                self.cfg.library.push(f.clone());
            }
            if !self.library.iter().any(|e| e.path == f) {
                self.library.push(LibEntry::new(f.clone()));
            }
            first.get_or_insert(f);
        }
        self.sort_library();
        first
    }

    fn sort_library(&mut self) {
        self.library.sort_by(|a, b| a.lower.cmp(&b.lower));
    }

    fn load(&mut self, path: PathBuf) {
        let (tx, rx) = mpsc::channel();
        let notify = self.notify.clone();
        let p2 = path.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Song::load(&p2));
            notify();
        });
        self.loading = Some((path, rx));
    }

    fn poll_background(&mut self) {
        if let Some(rx) = &self.scan_rx
            && let Ok(found) = rx.try_recv()
        {
            self.scan_rx = None;
            for p in found {
                if !self.library.iter().any(|e| e.path == p) {
                    self.library.push(LibEntry::new(p));
                }
            }
            self.sort_library();
        }
        if let Some((path, rx)) = &self.loading
            && let Ok(res) = rx.try_recv()
        {
            let path = path.clone();
            self.loading = None;
            match res {
                Ok(song) => {
                    let song = Arc::new(song);
                    self.muted = vec![false; song.tracks.len()];
                    self.engine.send(Cmd::Load(song.clone()));
                    self.song = Some(song);
                    self.cfg.last_file = Some(path);
                }
                Err(e) => {
                    self.engine.shared.log.warn(e.clone());
                    self.toast(e);
                }
            }
        }
    }

    fn sync_engine(&mut self) {
        let now = (self.cfg.player.clone(), self.cfg.keymap.clone(), self.cfg.drums.clone());
        if now != self.engine_snapshot {
            if (now.0.speed - self.engine_snapshot.0.speed).abs() > f32::EPSILON {
                self.engine.send(Cmd::SetSpeed(now.0.speed));
            }
            self.engine.send(Cmd::Configure(Box::new(engine_config(&self.cfg))));
            self.engine_snapshot = now;
        }
        // Speed changed by a hotkey → reflect in the UI and config.
        let s = self.engine.shared.speed();
        if (s - self.cfg.player.speed).abs() > 1e-4 {
            self.cfg.player.speed = s;
            self.engine_snapshot.0.speed = s;
        }

        let live_now = (self.cfg.live.clone(), self.cfg.keymap.clone(), self.cfg.drums.clone());
        if live_now != self.live_snapshot {
            if let Some(l) = &self.live {
                let k = &self.cfg.live.keys;
                l.update(
                    live_layout(&self.cfg),
                    k.mapper_options(self.cfg.live.layout),
                    k.fixed_hold.then(|| Duration::from_millis(u64::from(k.hold_ms))),
                );
            }
            self.live_snapshot = live_now;
        }

        if self.applied_hotkeys.as_ref() != Some(&self.cfg.hotkeys) {
            self.hotkey_problems = self.hotkeys.apply(&self.cfg.hotkeys);
            self.applied_hotkeys = Some(self.cfg.hotkeys.clone());
        }
    }

    fn autosave(&mut self) {
        if self.last_save_check.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_save_check = Instant::now();
        if self.cfg != self.saved_cfg {
            match self.cfg.save(&self.cfg_path) {
                Ok(()) => self.saved_cfg = self.cfg.clone(),
                Err(e) => self.engine.shared.log.warn(format!("Could not save settings: {e}")),
            }
        }
    }

    fn accent(&self) -> Color32 {
        theme::accent(self.cfg.ui.accent)
    }

    // ------------------------------------------------------------------ panels

    fn nav(&mut self, ui: &mut egui::Ui) {
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            let (r, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
            icons::paint(ui.painter(), r, Icon::Note, self.accent());
            ui.vertical(|ui| {
                ui.label(RichText::new("nanoMIDI").size(17.0).strong());
                ui.label(RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).small().weak());
            });
        });
        ui.add_space(18.0);
        let items = [
            (Page::Player, Icon::Play, "Player"),
            (Page::Live, Icon::Piano, "Live input"),
            (Page::Settings, Icon::Gear, "Settings"),
            (Page::About, Icon::Info, "About"),
        ];
        for (page, icon, label) in items {
            let selected = self.page == page;
            let style = Style {
                fill: selected.then(|| self.accent()),
                frameless: true,
                align_left: true,
                min_size: Vec2::new(ui.available_width(), 36.0),
                text_size: Some(14.5),
                ..Style::default()
            };
            if icon_button(ui, icon, label, style).clicked() {
                self.page = page;
                if page == Page::Live && self.midi_ins.is_empty() {
                    self.refresh_inputs();
                }
            }
        }
        ui.with_layout(ELayout::bottom_up(Align::Min), |ui| {
            ui.add_space(10.0);
            let h = &self.cfg.hotkeys;
            for (k, v) in [
                (&h.play, "play"),
                (&h.pause, "pause"),
                (&h.stop, "stop"),
                (&h.speed_up, "faster"),
                (&h.slow_down, "slower"),
            ]
            .into_iter()
            .rev()
            {
                if !k.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(k.to_uppercase()).monospace().small().strong());
                        ui.label(RichText::new(v).small().weak());
                    });
                }
            }
            ui.label(RichText::new("HOTKEYS").small().weak());
        });
    }

    fn bottom(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        let (active, layout) = if let Some(live) = self.live.as_ref().filter(|_| self.page == Page::Live) {
            (live.active.bits(), Some(live_layout(&self.cfg)))
        } else {
            let layout = match self.cfg.player.mode {
                OutputMode::PianoKeys => Some(Layout::piano(&self.cfg.keymap, self.cfg.player.keys.keys88)),
                OutputMode::DrumKeys => Some(Layout::drums(&self.cfg.drums)),
                OutputMode::MidiOut => None,
            };
            (self.engine.shared.active.bits(), layout)
        };
        piano::piano(ui, 78.0, active, layout.as_ref(), self.cfg.ui.key_labels, self.accent());
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let sh = &self.engine.shared;
            let (dot, text) = match sh.state() {
                PlayState::Stopped => (Color32::GRAY, "Stopped".to_string()),
                PlayState::Countdown => {
                    (Color32::from_rgb(245, 158, 11), format!("Starting in {:.1} s", sh.countdown()))
                }
                PlayState::Playing => (Color32::from_rgb(34, 197, 94), "Playing".to_string()),
                PlayState::Paused => (Color32::from_rgb(245, 158, 11), "Paused".to_string()),
            };
            let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 4.0, dot);
            ui.label(RichText::new(text).small());
            if let Some(s) = &self.song {
                ui.label(RichText::new(format!("·  {}", s.title)).small().weak());
            }
            if let Some(l) = &self.live {
                ui.label(RichText::new(format!("·  Live: {}", l.port)).small().color(self.accent()));
            }
            ui.with_layout(ELayout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(format!("{:.0}%", sh.speed() * 100.0)).small().weak());
            });
        });
    }

    fn library_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.heading("Library");
            ui.label(RichText::new(self.library.len().to_string()).weak());
            if self.scan_rx.is_some() {
                ui.spinner();
            }
        });
        let search = ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Search songs")
                .margin(egui::Margin { left: 28, right: 6, top: 5, bottom: 5 })
                .desired_width(f32::INFINITY),
        );
        let icon_rect =
            egui::Rect::from_center_size(search.rect.left_center() + Vec2::new(15.0, 0.0), Vec2::splat(16.0));
        icons::paint(ui.painter(), icon_rect, Icon::Search, ui.visuals().weak_text_color());
        ui.horizontal(|ui| {
            if icon_button(ui, Icon::Plus, "Files", Style::default()).on_hover_text("Add .mid / .midi files").clicked()
                && let Some(files) = rfd::FileDialog::new().add_filter("MIDI", &["mid", "midi"]).pick_files()
                && let Some(first) = self.add_files(files)
            {
                self.load(first);
            }
            if icon_button(ui, Icon::Folder, "Folder", Style::default())
                .on_hover_text("Watch a folder of MIDI files")
                .clicked()
                && let Some(dir) = rfd::FileDialog::new().pick_folder()
            {
                if !self.cfg.folders.contains(&dir) {
                    self.cfg.folders.push(dir);
                }
                self.rescan();
            }
            if icon_button(ui, Icon::Refresh, "", Style::default()).on_hover_text("Rescan folders").clicked() {
                self.library = self.cfg.library.iter().cloned().map(LibEntry::new).collect();
                self.rescan();
            }
        });
        ui.add_space(4.0);

        let q = self.search.to_lowercase();
        let rows: Vec<usize> =
            (0..self.library.len()).filter(|&i| q.is_empty() || self.library[i].lower.contains(&q)).collect();
        if self.library.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
                icons::paint(ui.painter(), r, Icon::Note, ui.visuals().weak_text_color());
                ui.label(RichText::new("Drop MIDI files here\nor click Files").weak());
            });
            return;
        }
        let current = self.song.as_ref().map(|s| s.path.clone());
        let loading = self.loading.as_ref().map(|l| l.0.clone());
        let mut to_load = None;
        let mut to_remove = None;
        let row_h = 30.0;
        egui::ScrollArea::vertical().auto_shrink(false).show_rows(ui, row_h, rows.len(), |ui, range| {
            for &i in &rows[range] {
                let e = &self.library[i];
                let selected = current.as_ref() == Some(&e.path);
                let is_loading = loading.as_ref() == Some(&e.path);
                let label = if is_loading { format!("{}  (loading…)", e.name) } else { e.name.clone() };
                let text =
                    if selected { RichText::new(label).color(Color32::WHITE).strong() } else { RichText::new(label) };
                let btn = egui::Button::new(text)
                    .fill(if selected { self.accent() } else { Color32::TRANSPARENT })
                    .min_size(Vec2::new(ui.available_width(), row_h - 4.0))
                    .truncate();
                let resp = ui.add(btn).on_hover_text(e.path.display().to_string());
                if resp.clicked() {
                    to_load = Some(e.path.clone());
                }
                resp.context_menu(|ui| {
                    if ui.button("Remove from library").clicked() {
                        to_remove = Some(i);
                        ui.close();
                    }
                });
            }
        });
        if let Some(p) = to_load {
            self.load(p);
        }
        if let Some(i) = to_remove {
            let e = self.library.remove(i);
            self.cfg.library.retain(|p| p != &e.path);
        }
    }

    // ------------------------------------------------------------------ pages

    fn player_page(&mut self, ui: &mut egui::Ui) {
        let accent = self.accent();
        let state = self.engine.shared.state();
        ui.add_space(8.0);

        // Header
        match &self.song {
            Some(s) => {
                ui.label(RichText::new(&s.title).size(24.0).strong());
                let tracks = s.tracks.iter().filter(|t| t.notes > 0).count();
                ui.label(
                    RichText::new(format!(
                        "{}  ·  {} notes  ·  {} track{}  ·  range {} – {}",
                        fmt_time(s.duration),
                        s.note_count,
                        tracks,
                        if tracks == 1 { "" } else { "s" },
                        note_name(s.lowest),
                        note_name(s.highest)
                    ))
                    .weak(),
                );
            }
            None => {
                ui.label(RichText::new("No song loaded").size(24.0).strong());
                ui.label(
                    RichText::new("Pick a song from the library, or drop a .mid file anywhere on the window.").weak(),
                );
            }
        }
        ui.add_space(8.0);

        // Transport card
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let duration = self.song.as_ref().map_or(0.0, |s| s.duration);
            let pos = self.seek_drag.unwrap_or_else(|| self.engine.shared.position());
            ui.horizontal(|ui| {
                ui.label(RichText::new(fmt_time(pos)).monospace());
                let mut v = pos;
                ui.spacing_mut().slider_width = (ui.available_width() - 60.0).max(80.0);
                let resp = ui.add_enabled(
                    duration > 0.0,
                    egui::Slider::new(&mut v, 0.0..=duration.max(0.001)).show_value(false),
                );
                if resp.dragged() || resp.changed() {
                    self.seek_drag = Some(v);
                }
                if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                    self.engine.send(Cmd::Seek(v));
                    self.seek_drag = None;
                }
                ui.label(RichText::new(fmt_time(duration)).monospace().weak());
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let playing = matches!(state, PlayState::Playing | PlayState::Countdown);
                let (icon, label) = if playing { (Icon::Pause, "Pause") } else { (Icon::Play, "Play") };
                let style = Style {
                    fill: Some(accent),
                    min_size: Vec2::new(124.0, 40.0),
                    text_size: Some(16.0),
                    strong: true,
                    ..Style::default()
                };
                let enabled = self.song.is_some();
                if ui.add_enabled_ui(enabled, |ui| icon_button(ui, icon, label, style)).inner.clicked() {
                    if playing || state == PlayState::Paused {
                        self.engine.send(Cmd::TogglePause);
                    } else {
                        self.engine.send(Cmd::Play { delay: self.cfg.player.start_delay });
                    }
                }
                let square = Style { min_size: Vec2::new(42.0, 40.0), text_size: Some(16.0), ..Style::default() };
                let stop = ui.add_enabled_ui(state != PlayState::Stopped, |ui| icon_button(ui, Icon::Stop, "", square));
                if stop.inner.on_hover_text("Stop").clicked() {
                    self.engine.send(Cmd::Stop);
                }
                let lp = &mut self.cfg.player.loop_song;
                if icon_button(ui, Icon::Loop, "", Style { selected: *lp, ..square })
                    .on_hover_text("Loop song")
                    .clicked()
                {
                    *lp = !*lp;
                }
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new("SPEED").small().weak());
                    ui.horizontal(|ui| {
                        let mut pct = self.cfg.player.speed * 100.0;
                        ui.spacing_mut().slider_width = 170.0;
                        if ui
                            .add(
                                egui::Slider::new(&mut pct, MIN_SPEED * 100.0..=MAX_SPEED * 100.0)
                                    .logarithmic(true)
                                    .suffix("%")
                                    .fixed_decimals(0),
                            )
                            .changed()
                        {
                            self.cfg.player.speed = (pct / 100.0).clamp(MIN_SPEED, MAX_SPEED);
                        }
                        if ui.small_button("1×").on_hover_text("Reset speed").clicked() {
                            self.cfg.player.speed = 1.0;
                        }
                    });
                });
            });
            if state == PlayState::Countdown {
                ui.label(
                    RichText::new(format!("Switch to your game: starting in {:.1} s", self.engine.shared.countdown()))
                        .color(accent),
                );
            }
        });
        ui.add_space(8.0);

        // Output card
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("OUTPUT").small().weak());
            ui.horizontal(|ui| {
                let m = &mut self.cfg.player.mode;
                seg(ui, m, OutputMode::PianoKeys, Icon::Piano, "Piano keys");
                seg(ui, m, OutputMode::DrumKeys, Icon::Drum, "Drum keys");
                if seg(ui, m, OutputMode::MidiOut, Icon::Midi, "MIDI device").clicked() {
                    self.refresh_outputs();
                }
            });
            match self.cfg.player.mode {
                OutputMode::MidiOut => {
                    ui.horizontal(|ui| {
                        let port = &mut self.cfg.player.midi_out_port;
                        egui::ComboBox::from_id_salt("midi_out")
                            .width(260.0)
                            .selected_text(if port.is_empty() { "Choose a device…" } else { port.as_str() })
                            .show_ui(ui, |ui| {
                                for p in &self.midi_outs {
                                    ui.selectable_value(port, p.clone(), p);
                                }
                                if self.midi_outs.is_empty() {
                                    ui.label(RichText::new("No MIDI outputs found").weak());
                                }
                            });
                        if icon_button(ui, Icon::Refresh, "", Style::default())
                            .on_hover_text("Refresh devices")
                            .clicked()
                        {
                            self.refresh_outputs();
                        }
                    });
                    ui.label(
                        RichText::new(
                            "Plays the file on a synth or DAW. All channels, instruments and controllers are sent.",
                        )
                        .small()
                        .weak(),
                    );
                }
                OutputMode::PianoKeys => self.fit_hint(ui),
                OutputMode::DrumKeys => {
                    ui.label(
                        RichText::new(
                            "General MIDI drum notes are typed with the drum kit keys (edit them in Settings).",
                        )
                        .small()
                        .weak(),
                    );
                }
            }
        });
        ui.add_space(8.0);

        if self.cfg.player.mode != OutputMode::MidiOut {
            let kind = if self.cfg.player.mode == OutputMode::DrumKeys { LayoutKind::Drums } else { LayoutKind::Piano };
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("KEYBOARD OPTIONS").small().weak());
                key_options(ui, &mut self.cfg.player.keys, kind, "player");
                ui.separator();
                egui::Grid::new("player_extra")
                    .num_columns(2)
                    .min_col_width(((ui.available_width() - 24.0) / 2.0).max(150.0))
                    .spacing([24.0, 8.0]).show(ui, |ui| {
                    switch(ui, &mut self.cfg.player.release_on_pause, "Release keys on pause", "Lift every held key when you pause");
                    ui.horizontal(|ui| {
                        ui.label("Start delay");
                        ui.add(egui::Slider::new(&mut self.cfg.player.start_delay, 0.0..=10.0).step_by(0.5).suffix(" s"))
                            .on_hover_text("Countdown after clicking Play, so you can focus the game window. Hotkeys always start instantly.");
                    });
                    ui.end_row();
                    let h = &mut self.cfg.player.humanize;
                    switch(ui, &mut h.enabled, "Humanize", "Add small timing mistakes and wrong notes");
                    ui.add_enabled_ui(h.enabled, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Timing");
                            ui.add(egui::DragValue::new(&mut h.timing_pct).range(0.0..=100.0).suffix("%").speed(0.2));
                            ui.label("Wrong notes");
                            ui.add(egui::DragValue::new(&mut h.wrong_note_pct).range(0.0..=100.0).suffix("%").speed(0.2));
                        });
                    });
                    ui.end_row();
                });
            });
            ui.add_space(8.0);
        }

        // Tracks
        if let Some(song) = self.song.clone() {
            let with_notes: Vec<usize> = (0..song.tracks.len()).filter(|&i| song.tracks[i].notes > 0).collect();
            if with_notes.len() > 1 {
                theme::card(ui).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::CollapsingHeader::new(RichText::new(format!("TRACKS ({})", with_notes.len())).small().weak())
                        .id_salt("tracks")
                        .show(ui, |ui| {
                            let mut changed = false;
                            for &i in &with_notes {
                                let t = &song.tracks[i];
                                let mut on = !self.muted[i];
                                let name = if t.name.is_empty() { format!("Track {}", i + 1) } else { t.name.clone() };
                                let drums = if t.drum_notes * 2 > t.notes { "  (drums)" } else { "" };
                                if ui.checkbox(&mut on, format!("{name}{drums}  ·  {} notes", t.notes)).changed() {
                                    self.muted[i] = !on;
                                    changed = true;
                                }
                            }
                            if changed {
                                self.engine.send(Cmd::SetMutedTracks(self.muted.clone()));
                            }
                        });
                });
                ui.add_space(8.0);
            }
        }

        log_card(ui, "log_player", &self.engine.shared.log);
    }

    /// How well the song fits the 61/88‑key layout, with a one‑click fix.
    fn fit_hint(&mut self, ui: &mut egui::Ui) {
        let Some(song) = self.song.clone() else { return };
        if song.note_count == 0 {
            return;
        }
        let k = self.cfg.player.keys.clone();
        let layout = Layout::piano(&self.cfg.keymap, k.keys88);
        let total = song.note_count as f32;
        let current = song.playable(&layout, i16::from(k.transpose()));
        let (best_oct, best) = (-2i8..=2)
            .map(|o| (o, song.playable(&layout, i16::from(o * 12 + k.key_shift))))
            .max_by_key(|&(o, n)| (n, -i32::from(o).abs()))
            .unwrap_or((0, current));
        ui.horizontal(|ui| {
            let pct = current as f32 / total * 100.0;
            let color = if pct >= 99.5 { Color32::from_rgb(34, 197, 94) } else { Color32::from_rgb(245, 158, 11) };
            ui.label(
                RichText::new(format!(
                    "{pct:.0}% of notes fit the {} layout",
                    if k.keys88 { "88-key" } else { "61-key" }
                ))
                .small()
                .color(color),
            );
            if best > current
                && ui.small_button(format!("Use octave {best_oct:+} ({:.0}%)", best as f32 / total * 100.0)).clicked()
            {
                self.cfg.player.keys.octave = best_oct;
            }
        });
    }

    fn live_page(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.label(RichText::new("Live input").size(24.0).strong());
        ui.label(
            RichText::new("Play a MIDI keyboard and nanoMIDI types the matching keys into the focused window.").weak(),
        );
        ui.add_space(8.0);
        let accent = self.accent();
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("MIDI INPUT").small().weak());
            ui.horizontal(|ui| {
                let connected = self.live.is_some();
                ui.add_enabled_ui(!connected, |ui| {
                    let port = &mut self.cfg.live.port;
                    egui::ComboBox::from_id_salt("midi_in")
                        .width(260.0)
                        .selected_text(if port.is_empty() { "Choose a device…" } else { port.as_str() })
                        .show_ui(ui, |ui| {
                            for p in &self.midi_ins {
                                ui.selectable_value(port, p.clone(), p);
                            }
                            if self.midi_ins.is_empty() {
                                ui.label(RichText::new("No MIDI inputs found").weak());
                            }
                        });
                });
                let refresh = ui.add_enabled_ui(!connected, |ui| icon_button(ui, Icon::Refresh, "", Style::default()));
                if refresh.inner.on_hover_text("Refresh devices").clicked() {
                    self.refresh_inputs();
                }
                let (icon, label, fill) = if connected {
                    (Icon::Stop, "Disconnect", Color32::from_rgb(220, 60, 70))
                } else {
                    (Icon::Play, "Connect", accent)
                };
                let style =
                    Style { fill: Some(fill), min_size: Vec2::new(130.0, 32.0), strong: true, ..Style::default() };
                let can = connected || !self.cfg.live.port.is_empty();
                if ui.add_enabled_ui(can, |ui| icon_button(ui, icon, label, style)).inner.clicked() {
                    if connected {
                        self.live = None;
                    } else {
                        let k = &self.cfg.live.keys;
                        match LiveInput::connect(
                            &self.cfg.live.port,
                            live_layout(&self.cfg),
                            k.mapper_options(self.cfg.live.layout),
                            k.fixed_hold.then(|| Duration::from_millis(u64::from(k.hold_ms))),
                            self.live_log.clone(),
                            self.notify.clone(),
                        ) {
                            Ok(l) => self.live = Some(l),
                            Err(e) => self.live_log.warn(e),
                        }
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Layout");
                seg(ui, &mut self.cfg.live.layout, LayoutKind::Piano, Icon::Piano, "Piano");
                seg(ui, &mut self.cfg.live.layout, LayoutKind::Drums, Icon::Drum, "Drums");
            });
        });
        ui.add_space(8.0);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("KEYBOARD OPTIONS").small().weak());
            key_options(ui, &mut self.cfg.live.keys, self.cfg.live.layout, "live");
        });
        ui.add_space(8.0);
        log_card(ui, "log_live", &self.live_log);
    }

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        ui.label(RichText::new("Settings").size(24.0).strong());
        ui.label(RichText::new("Changes are saved automatically.").weak());
        ui.add_space(8.0);

        // Hotkeys
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("GLOBAL HOTKEYS").small().weak());
            ui.label(RichText::new("Work system-wide, even while the game is focused.").small().weak());
            if let Some(action) = self.rebinding {
                let pressed = ui.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                        _ => None,
                    })
                });
                if let Some((key, mods)) = pressed {
                    if key != egui::Key::Escape
                        && let Some(s) = hotkey_string(key, mods)
                    {
                        *hotkey_slot(&mut self.cfg.hotkeys, action) = s;
                    }
                    self.rebinding = None;
                }
            }
            egui::Grid::new("hotkeys").num_columns(3).spacing([16.0, 6.0]).show(ui, |ui| {
                for (action, name) in [
                    (Action::Play, "Play / resume"),
                    (Action::Pause, "Pause / resume"),
                    (Action::Stop, "Stop"),
                    (Action::SpeedUp, "Speed up"),
                    (Action::SlowDown, "Slow down"),
                ] {
                    ui.label(name);
                    let waiting = self.rebinding == Some(action);
                    let current = hotkey_slot(&mut self.cfg.hotkeys, action).clone();
                    let text = if waiting {
                        RichText::new("Press a key…").color(self.accent())
                    } else if current.is_empty() {
                        RichText::new("Not set").weak()
                    } else {
                        RichText::new(current.to_uppercase()).monospace().strong()
                    };
                    if ui
                        .add(egui::Button::new(text).min_size(Vec2::new(140.0, 26.0)))
                        .on_hover_text("Click, then press a key (Esc cancels)")
                        .clicked()
                    {
                        self.rebinding = if waiting { None } else { Some(action) };
                    }
                    let small = Style {
                        frameless: true,
                        min_size: Vec2::new(26.0, 26.0),
                        text_size: Some(11.0),
                        ..Style::default()
                    };
                    if icon_button(ui, Icon::Close, "", small).on_hover_text("Clear").clicked() {
                        hotkey_slot(&mut self.cfg.hotkeys, action).clear();
                    }
                    ui.end_row();
                }
            });
            for p in &self.hotkey_problems {
                warn_label(ui, p);
            }
            ui.horizontal(|ui| {
                ui.label("Speed step");
                let mut pct = self.cfg.player.speed_step * 100.0;
                if ui.add(egui::Slider::new(&mut pct, 1.0..=50.0).suffix("%").fixed_decimals(0)).changed() {
                    self.cfg.player.speed_step = pct / 100.0;
                }
                if ui.small_button("Reset").clicked() {
                    self.cfg.hotkeys = crate::config::Hotkeys::default();
                    self.cfg.player.speed_step = 0.05;
                }
            });
        });
        ui.add_space(8.0);

        // Appearance
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("APPEARANCE").small().weak());
            ui.horizontal(|ui| {
                ui.label("Theme");
                let t = &mut self.cfg.ui.theme;
                ui.selectable_value(t, ThemePref::System, "System");
                ui.selectable_value(t, ThemePref::Dark, "Dark");
                ui.selectable_value(t, ThemePref::Light, "Light");
            });
            ui.horizontal(|ui| {
                ui.label("Accent");
                for (name, rgb) in theme::ACCENTS {
                    let (r, resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
                    let selected = self.cfg.ui.accent == rgb;
                    ui.painter().circle_filled(r.center(), if selected { 10.0 } else { 8.0 }, theme::accent(rgb));
                    if selected {
                        ui.painter().circle_stroke(
                            r.center(),
                            10.5,
                            egui::Stroke::new(2.0, ui.visuals().strong_text_color()),
                        );
                    }
                    if resp.on_hover_text(name).clicked() {
                        self.cfg.ui.accent = rgb;
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Interface size");
                let mut z = self.cfg.ui.zoom * 100.0;
                let r = ui.add(egui::Slider::new(&mut z, 75.0..=200.0).suffix("%").fixed_decimals(0));
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    self.cfg.ui.zoom = z / 100.0;
                }
            });
            switch(ui, &mut self.cfg.ui.always_on_top, "Always on top", "Keep the window above the game");
            switch(ui, &mut self.cfg.ui.key_labels, "Key labels on piano", "Show which key each note types");
        });
        ui.add_space(8.0);

        // Piano layout
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("PIANO LAYOUT").small().weak());
            ui.label(
                RichText::new("One character per note, lowest first. Capitals and symbols are typed with Shift.")
                    .small()
                    .weak(),
            );
            let km = &mut self.cfg.keymap;
            egui::Grid::new("keymap").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                for (label, value, hint) in [
                    ("61 keys (C2–C7)", &mut km.keys61, "notes 36–96"),
                    ("Low notes (Ctrl)", &mut km.keys88_low, "notes 21–35, 88-key mode"),
                    ("High notes (Ctrl)", &mut km.keys88_high, "notes 97–108, 88-key mode"),
                    ("Velocity (Alt)", &mut km.velocity_keys, "32 levels, soft to loud"),
                ] {
                    ui.label(label).on_hover_text(hint);
                    ui.add(
                        egui::TextEdit::singleline(value).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY),
                    );
                    ui.end_row();
                }
            });
            for p in km.problems() {
                warn_label(ui, &p);
            }
            if ui.button("Reset layout").clicked() {
                *km = KeyMapConfig::default();
            }
        });
        ui.add_space(8.0);

        // Drum kit
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::CollapsingHeader::new(RichText::new("DRUM KIT").small().weak()).id_salt("drums").show(ui, |ui| {
                egui::Grid::new("drums_grid").num_columns(3).striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
                    for pad in &mut self.cfg.drums {
                        ui.label(&pad.name);
                        ui.label(RichText::new(format!("note {} ({})", pad.note, note_name(pad.note))).weak().small());
                        let mut s = pad.key.to_string();
                        if ui
                            .add(
                                egui::TextEdit::singleline(&mut s)
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(28.0)
                                    .char_limit(2),
                            )
                            .changed()
                            && let Some(c) = s.chars().find(|c| *c != pad.key).or_else(|| s.chars().next())
                        {
                            pad.key = c;
                        }
                        ui.end_row();
                    }
                });
                if ui.button("Reset drum kit").clicked() {
                    self.cfg.drums = keymap::default_drum_pads();
                }
            });
        });
        ui.add_space(8.0);

        // Folders
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("LIBRARY FOLDERS").small().weak());
            let mut remove = None;
            for (i, f) in self.cfg.folders.iter().enumerate() {
                ui.horizontal(|ui| {
                    let small = Style {
                        frameless: true,
                        min_size: Vec2::new(26.0, 26.0),
                        text_size: Some(11.0),
                        ..Style::default()
                    };
                    if icon_button(ui, Icon::Close, "", small).on_hover_text("Remove folder").clicked() {
                        remove = Some(i);
                    }
                    ui.label(RichText::new(f.display().to_string()).monospace());
                });
            }
            if let Some(i) = remove {
                self.cfg.folders.remove(i);
                self.library = self.cfg.library.iter().cloned().map(LibEntry::new).collect();
                self.rescan();
            }
            if self.cfg.folders.is_empty() {
                ui.label(
                    RichText::new("No folders. Add one to have its MIDI files appear in the library.").small().weak(),
                );
            }
            if icon_button(ui, Icon::Plus, "Add folder", Style::default()).clicked()
                && let Some(dir) = rfd::FileDialog::new().pick_folder()
            {
                if !self.cfg.folders.contains(&dir) {
                    self.cfg.folders.push(dir);
                }
                self.rescan();
            }
        });
        ui.add_space(8.0);

        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("CONFIG FILE").small().weak());
            ui.label(RichText::new(self.cfg_path.display().to_string()).monospace().small());
            if ui.button("Restore all defaults").on_hover_text("Keeps your library").clicked() {
                let library = std::mem::take(&mut self.cfg.library);
                let folders = std::mem::take(&mut self.cfg.folders);
                self.cfg = Config { library, folders, last_file: self.cfg.last_file.clone(), ..Config::default() };
                self.toast("Settings restored to defaults");
            }
        });
    }

    fn about_page(ui: &mut egui::Ui, accent: Color32) {
        ui.add_space(8.0);
        ui.label(RichText::new("nanoMIDI").size(28.0).strong().color(accent));
        ui.label(RichText::new(concat!("Version ", env!("CARGO_PKG_VERSION"))).weak());
        ui.add_space(8.0);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label("A fast, native MIDI player for virtual pianos and drum kits. It turns MIDI files, or a live MIDI keyboard, into keystrokes, or plays them on any MIDI device.");
            ui.add_space(6.0);
            for line in [
                "Native Rust app: starts instantly, small download, low memory use",
                "Drift-free timing: every note is scheduled against its absolute time in the file",
                "Seek, loop, speed 10–500% and per-track mute",
                "61 or 88 key layouts, sustain, velocity, finger limit, fixed hold length",
                "Live MIDI keyboard to QWERTY, with the same options",
                "MIDI device output with instruments, controllers and pitch bend",
                "No telemetry, no network access",
            ] {
                ui.label(format!("•  {line}"));
            }
        });
        ui.add_space(8.0);
        ui.label(RichText::new("Inspired by nanoMIDIPlayer by NotHammer043.").small().weak());
    }
}

fn hotkey_slot(h: &mut crate::config::Hotkeys, a: Action) -> &mut String {
    match a {
        Action::Play => &mut h.play,
        Action::Pause => &mut h.pause,
        Action::Stop => &mut h.stop,
        Action::SpeedUp => &mut h.speed_up,
        Action::SlowDown => &mut h.slow_down,
    }
}

/// Segmented‑control style choice button.
fn seg<T: PartialEq + Copy>(ui: &mut egui::Ui, current: &mut T, value: T, icon: Icon, label: &str) -> egui::Response {
    let r = icon_button(ui, icon, label, Style { selected: *current == value, ..Style::default() });
    if r.clicked() {
        *current = value;
    }
    r
}

/// iOS‑style switch with a label.
fn switch(ui: &mut egui::Ui, on: &mut bool, label: &str, hint: &str) -> egui::Response {
    let resp = ui
        .horizontal(|ui| {
            let size = Vec2::new(34.0, 18.0);
            let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
            if resp.clicked() {
                *on = !*on;
                resp.mark_changed();
            }
            let t = ui.ctx().animate_bool_responsive(resp.id, *on);
            let v = ui.visuals();
            let off_bg = if v.dark_mode { Color32::from_gray(70) } else { Color32::from_gray(190) };
            let bg = off_bg.lerp_to_gamma(v.selection.stroke.color, t);
            ui.painter().rect_filled(rect, 9.0, bg);
            let x = egui::lerp(rect.left() + 9.0..=rect.right() - 9.0, t);
            ui.painter().circle_filled(egui::pos2(x, rect.center().y), 7.0, Color32::WHITE);
            let l = ui.add(egui::Label::new(label).sense(Sense::click()));
            if l.clicked() {
                *on = !*on;
                resp.mark_changed();
            }
            resp | l
        })
        .inner;
    if hint.is_empty() { resp } else { resp.on_hover_text(hint) }
}

fn key_options(ui: &mut egui::Ui, k: &mut KeyOptions, kind: LayoutKind, id: &str) {
    ui.spacing_mut().slider_width = 110.0;
    let col = ((ui.available_width() - 24.0) / 2.0).max(150.0);
    egui::Grid::new(format!("keyopts_{id}")).num_columns(2).min_col_width(col).spacing([24.0, 8.0]).show(ui, |ui| {
        if kind == LayoutKind::Piano {
            switch(ui, &mut k.sustain, "Sustain pedal (Space)", "Hold Space while the sustain pedal is down");
            switch(
                ui,
                &mut k.no_doubles,
                "No doubles",
                "Re-press a key when two notes share it, so repeated notes are heard",
            );
            ui.end_row();
            switch(
                ui,
                &mut k.velocity,
                "Velocity (Alt + key)",
                "Send note loudness before each note (for pianos that support it)",
            );
            switch(ui, &mut k.keys88, "88 keys (Ctrl)", "Play the lowest and highest notes with Ctrl + key");
            ui.end_row();
            ui.horizontal(|ui| {
                ui.label("Octave");
                egui::ComboBox::from_id_salt(format!("oct_{id}"))
                    .width(60.0)
                    .selected_text(format!("{:+}", k.octave))
                    .show_ui(ui, |ui| {
                        for o in -2..=2 {
                            ui.selectable_value(&mut k.octave, o, format!("{o:+}"));
                        }
                    });
                ui.label("Key");
                egui::ComboBox::from_id_salt(format!("key_{id}"))
                    .width(60.0)
                    .selected_text(NOTE_NAMES[k.key_shift.clamp(0, 11) as usize])
                    .show_ui(ui, |ui| {
                        for (i, n) in NOTE_NAMES.iter().enumerate() {
                            ui.selectable_value(&mut k.key_shift, i as i8, *n);
                        }
                    });
            })
            .response
            .on_hover_text("Transpose: octave shift plus semitones (key)");
            ui.add_enabled_ui(k.sustain, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Pedal threshold");
                    ui.add(egui::DragValue::new(&mut k.sustain_cutoff).range(0..=126));
                });
            });
            ui.end_row();
        }
        ui.horizontal(|ui| {
            ui.label("Finger limit");
            ui.add(
                egui::Slider::new(&mut k.finger_limit, 0..=10)
                    .custom_formatter(|v, _| if v == 0.0 { "∞".into() } else { format!("{v:.0}") }),
            )
            .on_hover_text("Maximum keys held at once (∞ = no limit)");
        });
        ui.horizontal(|ui| {
            switch(
                ui,
                &mut k.fixed_hold,
                "Fixed hold",
                "Release every key after a fixed time instead of at the note's end",
            );
            ui.add_enabled(k.fixed_hold, egui::DragValue::new(&mut k.hold_ms).range(1..=10_000).suffix(" ms"));
        });
        ui.end_row();
    });
}

fn log_card(ui: &mut egui::Ui, id: &str, log: &Log) {
    theme::card(ui).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::CollapsingHeader::new(RichText::new("ACTIVITY").small().weak()).id_salt(id).default_open(true).show(
            ui,
            |ui| {
                let lines = log.lines();
                egui::ScrollArea::vertical()
                    .id_salt(id)
                    .max_height(120.0)
                    .stick_to_bottom(true)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        if lines.is_empty() {
                            ui.label(RichText::new("Nothing yet").weak().small());
                        }
                        for (warn, l) in lines {
                            if warn {
                                warn_label(ui, &l);
                            } else {
                                ui.label(RichText::new(l).monospace().small());
                            }
                        }
                    });
                if ui.small_button("Clear").clicked() {
                    log.clear();
                }
            },
        );
    });
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let t = (self.cfg.ui.theme, self.cfg.ui.accent, self.cfg.ui.zoom);
        if self.applied_theme != Some(t) {
            theme::apply(&ctx, t.0, t.1, t.2);
            self.applied_theme = Some(t);
        }
        if self.applied_on_top != Some(self.cfg.ui.always_on_top) {
            let level =
                if self.cfg.ui.always_on_top { egui::WindowLevel::AlwaysOnTop } else { egui::WindowLevel::Normal };
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
            self.applied_on_top = Some(self.cfg.ui.always_on_top);
        }

        self.poll_background();

        // Drag & drop
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect()
        });
        if !dropped.is_empty() {
            let mut files = Vec::new();
            for p in dropped {
                if p.is_dir() {
                    files.extend(scan_folders(vec![p]));
                } else {
                    files.push(p);
                }
            }
            if let Some(first) = self.add_files(files) {
                self.load(first);
                self.page = Page::Player;
            }
        }

        // Space toggles play/pause when no text field has focus.
        if self.rebinding.is_none()
            && !ctx.egui_wants_keyboard_input()
            && ctx.input(|i| i.key_pressed(egui::Key::Space))
            && self.song.is_some()
        {
            match self.engine.shared.state() {
                PlayState::Stopped => self.engine.send(Cmd::Play { delay: self.cfg.player.start_delay }),
                _ => self.engine.send(Cmd::TogglePause),
            }
        }

        let nav_fill =
            if ui.visuals().dark_mode { Color32::from_rgb(13, 14, 17) } else { Color32::from_rgb(234, 236, 241) };
        egui::Panel::left("nav")
            .resizable(false)
            .exact_size(176.0)
            .frame(egui::Frame::new().fill(nav_fill).inner_margin(egui::Margin::symmetric(10, 0)))
            .show(ui, |ui| self.nav(ui));
        egui::Panel::bottom("piano")
            .resizable(false)
            .frame(egui::Frame::new().fill(ui.visuals().panel_fill).inner_margin(egui::Margin::symmetric(12, 4)))
            .show(ui, |ui| self.bottom(ui));
        if self.page == Page::Player {
            egui::Panel::left("library")
                .default_size(250.0)
                .min_size(190.0)
                .max_size(420.0)
                .frame(egui::Frame::new().fill(ui.visuals().panel_fill).inner_margin(egui::Margin::symmetric(12, 0)))
                .show(ui, |ui| self.library_panel(ui));
        }
        let accent = self.accent();
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(ui.visuals().widgets.noninteractive.bg_fill)
                    .inner_margin(egui::Margin::symmetric(18, 6)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| match self.page {
                    Page::Player => self.player_page(ui),
                    Page::Live => self.live_page(ui),
                    Page::Settings => self.settings_page(ui),
                    Page::About => Self::about_page(ui, accent),
                });
            });

        // Drop overlay
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            let rect = ctx.content_rect();
            let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("drop")));
            p.rect_filled(rect, 0.0, Color32::from_black_alpha(160));
            p.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Drop MIDI files to add them",
                egui::FontId::proportional(24.0),
                Color32::WHITE,
            );
        }

        // Toast
        if let Some((msg, at)) = &self.toast {
            if at.elapsed() < Duration::from_secs(4) {
                egui::Area::new(egui::Id::unique("toast")).anchor(egui::Align2::CENTER_TOP, [0.0, 16.0]).show(
                    &ctx,
                    |ui| {
                        theme::card(ui).show(ui, |ui| ui.label(msg.as_str()));
                    },
                );
                ctx.request_repaint_after(Duration::from_millis(250));
            } else {
                self.toast = None;
            }
        }

        self.sync_engine();
        self.autosave();

        if matches!(self.engine.shared.state(), PlayState::Playing | PlayState::Countdown) {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.live = None;
        self.engine.send(Cmd::Stop);
        let _ = self.cfg.save(&self.cfg_path);
    }
}
