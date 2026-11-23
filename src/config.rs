//! Persistent settings (JSON in the platform config directory).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::keymap::{DrumPad, KeyMapConfig, LayoutKind, default_drum_pads};
use crate::mapper::MapperOptions;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputMode {
    /// Simulated QWERTY keystrokes using the piano layout.
    PianoKeys,
    /// Simulated QWERTY keystrokes using the drum layout.
    DrumKeys,
    /// Send to a MIDI output port.
    MidiOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemePref {
    System,
    Dark,
    Light,
}

/// Keyboard‑simulation options shared by file playback and live input.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyOptions {
    pub sustain: bool,
    pub sustain_cutoff: u8,
    pub no_doubles: bool,
    pub velocity: bool,
    pub keys88: bool,
    /// Octave shift, −2‥+2.
    pub octave: i8,
    /// Key shift in semitones, 0‥11 (C‥B).
    pub key_shift: i8,
    /// 0 = unlimited.
    pub finger_limit: u8,
    /// Release each key after a fixed time instead of at note‑off.
    pub fixed_hold: bool,
    pub hold_ms: u32,
}

impl Default for KeyOptions {
    fn default() -> Self {
        Self {
            sustain: false,
            sustain_cutoff: 63,
            no_doubles: true,
            velocity: false,
            keys88: true,
            octave: 0,
            key_shift: 0,
            finger_limit: 0,
            fixed_hold: false,
            hold_ms: 100,
        }
    }
}

impl KeyOptions {
    pub fn transpose(&self) -> i8 {
        self.octave * 12 + self.key_shift
    }

    pub fn mapper_options(&self, kind: LayoutKind) -> MapperOptions {
        match kind {
            LayoutKind::Piano => MapperOptions {
                sustain: self.sustain,
                sustain_cutoff: self.sustain_cutoff,
                no_doubles: self.no_doubles,
                velocity: self.velocity,
                transpose: self.transpose(),
                finger_limit: self.finger_limit,
            },
            LayoutKind::Drums => MapperOptions {
                sustain: false,
                sustain_cutoff: 63,
                no_doubles: true,
                velocity: false,
                transpose: 0,
                finger_limit: self.finger_limit,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Humanize {
    pub enabled: bool,
    /// Chance (%) that a gap between notes is stretched/squeezed by up to 50 %.
    pub timing_pct: f32,
    /// Chance (%) that a note is played up to an octave off.
    pub wrong_note_pct: f32,
}

impl Default for Humanize {
    fn default() -> Self {
        Self { enabled: false, timing_pct: 5.0, wrong_note_pct: 2.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerConfig {
    pub mode: OutputMode,
    pub midi_out_port: String,
    pub speed: f32,
    /// Step used by the speed hotkeys (fraction, 0.05 = 5 %).
    pub speed_step: f32,
    pub loop_song: bool,
    pub release_on_pause: bool,
    /// Seconds to wait after pressing Play in the window (time to focus the game).
    pub start_delay: f32,
    pub keys: KeyOptions,
    pub humanize: Humanize,
}

impl Default for PlayerConfig {
    fn default() -> Self {
        Self {
            mode: OutputMode::PianoKeys,
            midi_out_port: String::new(),
            speed: 1.0,
            speed_step: 0.05,
            loop_song: false,
            release_on_pause: true,
            start_delay: 0.0,
            keys: KeyOptions::default(),
            humanize: Humanize::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LiveConfig {
    pub port: String,
    pub layout: LayoutKind,
    pub keys: KeyOptions,
}

impl Default for LiveConfig {
    fn default() -> Self {
        Self { port: String::new(), layout: LayoutKind::Piano, keys: KeyOptions::default() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub play: String,
    pub pause: String,
    pub stop: String,
    pub speed_up: String,
    pub slow_down: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self { play: "F1".into(), pause: "F2".into(), stop: "F3".into(), speed_up: "F4".into(), slow_down: "F5".into() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub theme: ThemePref,
    pub accent: [u8; 3],
    pub always_on_top: bool,
    pub key_labels: bool,
    pub zoom: f32,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self { theme: ThemePref::System, accent: [124, 92, 255], always_on_top: false, key_labels: true, zoom: 1.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub library: Vec<PathBuf>,
    pub folders: Vec<PathBuf>,
    pub last_file: Option<PathBuf>,
    pub player: PlayerConfig,
    pub live: LiveConfig,
    pub keymap: KeyMapConfig,
    pub drums: Vec<DrumPad>,
    pub hotkeys: Hotkeys,
    pub ui: UiConfig,
}

impl Default for Config {
    fn default() -> Self {
        // The original app keeps MIDIs here; pick them up automatically.
        let folders = dirs::document_dir()
            .map(|d| d.join("nanoMIDIPlayer").join("Midis"))
            .filter(|d| d.is_dir())
            .into_iter()
            .collect();
        Self {
            library: Vec::new(),
            folders,
            last_file: None,
            player: PlayerConfig::default(),
            live: LiveConfig::default(),
            keymap: KeyMapConfig::default(),
            drums: default_drum_pads(),
            hotkeys: Hotkeys::default(),
            ui: UiConfig::default(),
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("nanomidi").join("config.json")
    }

    /// Load, falling back to defaults (and keeping a backup of a broken file).
    pub fn load(path: &Path) -> (Config, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<Config>(&text) {
                Ok(mut c) => {
                    c.sanitize();
                    (c, None)
                }
                Err(e) => {
                    let backup = path.with_extension("json.bak");
                    let _ = std::fs::copy(path, &backup);
                    (
                        Config::default(),
                        Some(format!(
                            "Config was invalid ({e}); reset to defaults, old file saved as {}",
                            backup.display()
                        )),
                    )
                }
            },
            Err(_) => (Config::default(), None),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        // Write atomically so a crash never leaves a half‑written file.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    fn sanitize(&mut self) {
        let p = &mut self.player;
        p.speed = p.speed.clamp(0.1, 5.0);
        p.speed_step = p.speed_step.clamp(0.01, 1.0);
        p.start_delay = p.start_delay.clamp(0.0, 10.0);
        for k in [&mut self.player.keys, &mut self.live.keys] {
            k.octave = k.octave.clamp(-2, 2);
            k.key_shift = k.key_shift.clamp(0, 11);
            k.finger_limit = k.finger_limit.min(10);
            k.sustain_cutoff = k.sustain_cutoff.min(127);
            k.hold_ms = k.hold_ms.clamp(1, 10_000);
        }
        self.ui.zoom = self.ui.zoom.clamp(0.75, 2.0);
        self.library.retain(|p| p.is_file());
        self.library.dedup();
        if self.drums.is_empty() {
            self.drums = default_drum_pads();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_partial_files() {
        let dir = std::env::temp_dir().join(format!("nanomidi-test-{}", std::process::id()));
        let path = dir.join("config.json");
        let mut c = Config::default();
        c.player.speed = 1.5;
        c.save(&path).expect("save");
        let (loaded, warn) = Config::load(&path);
        assert!(warn.is_none());
        assert_eq!(loaded.player.speed, 1.5);

        // Missing fields fall back to defaults.
        std::fs::write(&path, r#"{"player":{"loop_song":true}}"#).expect("write");
        let (loaded, warn) = Config::load(&path);
        assert!(warn.is_none());
        assert!(loaded.player.loop_song);
        assert_eq!(loaded.hotkeys, Hotkeys::default());

        std::fs::write(&path, "{nope").expect("write");
        let (_, warn) = Config::load(&path);
        assert!(warn.is_some());
        let _ = std::fs::remove_dir_all(dir);
    }
}
