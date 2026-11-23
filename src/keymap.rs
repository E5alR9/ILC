//! Note → keystroke layouts for virtual pianos and drum kits.
//!
//! The defaults match the 61‑key layout used by most online / in‑game virtual
//! pianos (notes 36‥96), plus the common "88‑key" extension where the lowest 15
//! and highest 12 notes are sent with Ctrl held.

use serde::{Deserialize, Serialize};

/// Lowest note of the standard 61‑key layout (C2).
pub const KEYS61_FIRST: u8 = 36;
/// Notes 36‥96 (inclusive) as typed on a US keyboard. Upper‑case letters and
/// shifted digits are sent as Shift + base key.
pub const DEFAULT_KEYS61: &str = "1!2@34$5%6^78*9(0qQwWeErtTyYuiIoOpPasSdDfgGhHjJklLzZxcCvVbBnm";
/// Notes 21‥35, sent with Ctrl held when 88‑key mode is on.
pub const KEYS88_LOW_FIRST: u8 = 21;
pub const DEFAULT_KEYS88_LOW: &str = "1234567890qwert";
/// Notes 97‥108, sent with Ctrl held when 88‑key mode is on.
pub const KEYS88_HIGH_FIRST: u8 = 97;
pub const DEFAULT_KEYS88_HIGH: &str = "yuiopasdfghj";
/// 32 velocity buckets (0‑3, 4‑7, …, 124‑127), sent as Alt + key before a note.
pub const DEFAULT_VELOCITY_KEYS: &str = "1234567890qwertyuiopasdfghjklzxc";

/// A physical key the simulator can press.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Shift,
    Ctrl,
    Alt,
    Space,
}

/// One key transition sent to the operating system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyEvent {
    Down(Key),
    Up(Key),
}

/// Modifier that has to be held while the base key goes down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    None,
    Shift,
    Ctrl,
}

/// How a single note is played.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stroke {
    /// Un‑shifted key (lower‑case letter, digit, …).
    pub base: char,
    pub modifier: Modifier,
    /// Character the user wrote in the layout (for labels), e.g. `'Q'` or `'!'`.
    pub label: char,
}

/// Map a US‑layout shifted symbol back to the key that produces it.
fn unshift(c: char) -> Option<char> {
    Some(match c {
        '!' => '1',
        '@' => '2',
        '#' => '3',
        '$' => '4',
        '%' => '5',
        '^' => '6',
        '&' => '7',
        '*' => '8',
        '(' => '9',
        ')' => '0',
        '_' => '-',
        '+' => '=',
        '{' => '[',
        '}' => ']',
        '|' => '\\',
        ':' => ';',
        '"' => '\'',
        '<' => ',',
        '>' => '.',
        '?' => '/',
        '~' => '`',
        _ => return None,
    })
}

/// Turn a layout character into a stroke.
pub fn stroke_for(c: char) -> Stroke {
    if c.is_uppercase() {
        let base = c.to_lowercase().next().unwrap_or(c);
        Stroke { base, modifier: Modifier::Shift, label: c }
    } else if let Some(base) = unshift(c) {
        Stroke { base, modifier: Modifier::Shift, label: c }
    } else {
        Stroke { base: c, modifier: Modifier::None, label: c }
    }
}

/// User‑editable layout strings (persisted in the config file).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyMapConfig {
    pub keys61: String,
    pub keys88_low: String,
    pub keys88_high: String,
    pub velocity_keys: String,
}

impl Default for KeyMapConfig {
    fn default() -> Self {
        Self {
            keys61: DEFAULT_KEYS61.into(),
            keys88_low: DEFAULT_KEYS88_LOW.into(),
            keys88_high: DEFAULT_KEYS88_HIGH.into(),
            velocity_keys: DEFAULT_VELOCITY_KEYS.into(),
        }
    }
}

impl KeyMapConfig {
    /// Problems with the strings, if any (shown in Settings).
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let check = |name: &str, s: &str, want: usize, out: &mut Vec<String>| {
            let n = s.chars().count();
            if n != want {
                out.push(format!("{name}: {n} keys, expected {want}"));
            }
            if s.chars().any(char::is_whitespace) {
                out.push(format!("{name}: contains whitespace"));
            }
        };
        check("61-key layout", &self.keys61, 61, &mut out);
        check("88-key low notes", &self.keys88_low, 15, &mut out);
        check("88-key high notes", &self.keys88_high, 12, &mut out);
        check("Velocity keys", &self.velocity_keys, 32, &mut out);
        out
    }
}

/// A drum sound name, the General‑MIDI note that triggers it, and its key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DrumPad {
    pub name: String,
    pub note: u8,
    pub key: char,
}

/// Default drum kit (General MIDI percussion numbers).
pub fn default_drum_pads() -> Vec<DrumPad> {
    [
        ("Kick", 35, 'b'),
        ("Kick 2", 36, 'n'),
        ("Snare side stick", 37, 's'),
        ("Snare", 38, 'c'),
        ("Cowbell (hand clap)", 39, 'a'),
        ("Snare 2", 40, 'v'),
        ("Closed hi-hat", 42, 'x'),
        ("Pedal hi-hat", 44, 'm'),
        ("Low tom", 45, 'g'),
        ("Open hi-hat", 46, 'z'),
        ("Low-mid tom", 47, 'd'),
        ("Tom 1", 48, 'd'),
        ("Right crash", 49, 'r'),
        ("Tom 1 (2)", 50, 'f'),
        ("Ride", 51, 'u'),
        ("China crash", 52, 'o'),
        ("Ride bell", 53, 'i'),
        ("Left crash", 55, 'e'),
        ("Splash crash", 57, 'w'),
        ("Tom", 60, 'g'),
        ("Tom 2", 62, 'h'),
    ]
    .into_iter()
    .map(|(name, note, key)| DrumPad { name: name.into(), note, key })
    .collect()
}

/// Which layout a mapper uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayoutKind {
    Piano,
    Drums,
}

/// Resolved, constant‑time note → stroke table.
#[derive(Clone, Debug)]
pub struct Layout {
    pub strokes: [Option<Stroke>; 128],
    pub velocity: Vec<char>,
}

impl Layout {
    pub fn piano(cfg: &KeyMapConfig, keys88: bool) -> Self {
        let mut strokes = [None; 128];
        for (i, c) in cfg.keys61.chars().take(61).enumerate() {
            strokes[KEYS61_FIRST as usize + i] = Some(stroke_for(c));
        }
        if keys88 {
            let ctrl = |c: char| {
                let base = c.to_lowercase().next().unwrap_or(c);
                Some(Stroke { base, modifier: Modifier::Ctrl, label: c })
            };
            for (i, c) in cfg.keys88_low.chars().take(15).enumerate() {
                strokes[KEYS88_LOW_FIRST as usize + i] = ctrl(c);
            }
            for (i, c) in cfg.keys88_high.chars().take(12).enumerate() {
                strokes[KEYS88_HIGH_FIRST as usize + i] = ctrl(c);
            }
        }
        Self { strokes, velocity: cfg.velocity_keys.chars().collect() }
    }

    pub fn drums(pads: &[DrumPad]) -> Self {
        let mut strokes = [None; 128];
        for p in pads {
            if let Some(slot) = strokes.get_mut(p.note as usize) {
                *slot = Some(stroke_for(p.key));
            }
        }
        Self { strokes, velocity: Vec::new() }
    }

    /// Key used to announce a velocity (floor bucket of width 4).
    pub fn velocity_key(&self, velocity: u8) -> Option<char> {
        if self.velocity.is_empty() {
            return None;
        }
        let idx = (velocity as usize / 4).min(self.velocity.len() - 1);
        self.velocity.get(idx).copied()
    }

    pub fn get(&self, note: u8) -> Option<Stroke> {
        self.strokes.get(note as usize).copied().flatten()
    }
}

pub const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// "C4"‑style name (MIDI 60 = C4).
pub fn note_name(note: u8) -> String {
    let octave = i32::from(note) / 12 - 1;
    format!("{}{}", NOTE_NAMES[note as usize % 12], octave)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_covers_61_keys() {
        let l = Layout::piano(&KeyMapConfig::default(), false);
        assert_eq!(l.get(35), None);
        assert_eq!(l.get(36), Some(Stroke { base: '1', modifier: Modifier::None, label: '1' }));
        assert_eq!(l.get(37), Some(Stroke { base: '1', modifier: Modifier::Shift, label: '!' }));
        assert_eq!(l.get(54), Some(Stroke { base: 'q', modifier: Modifier::Shift, label: 'Q' }));
        assert_eq!(l.get(60).map(|s| s.base), Some('t'));
        assert_eq!(l.get(96).map(|s| s.base), Some('m'));
        assert_eq!(l.get(97), None);
        assert_eq!((36..=96).filter(|&n| l.get(n).is_some()).count(), 61);
    }

    #[test]
    fn layout_88_uses_ctrl() {
        let l = Layout::piano(&KeyMapConfig::default(), true);
        assert_eq!(l.get(21), Some(Stroke { base: '1', modifier: Modifier::Ctrl, label: '1' }));
        assert_eq!(l.get(35).map(|s| s.base), Some('t'));
        assert_eq!(l.get(97).map(|s| s.base), Some('y'));
        assert_eq!(l.get(108).map(|s| s.base), Some('j'));
        assert_eq!((0..128).filter(|&n| l.get(n).is_some()).count(), 88);
    }

    #[test]
    fn velocity_buckets_floor() {
        let l = Layout::piano(&KeyMapConfig::default(), false);
        assert_eq!(l.velocity_key(0), Some('1'));
        assert_eq!(l.velocity_key(3), Some('1'));
        assert_eq!(l.velocity_key(4), Some('2'));
        assert_eq!(l.velocity_key(5), Some('2'));
        assert_eq!(l.velocity_key(127), Some('c'));
    }

    #[test]
    fn default_config_has_no_problems() {
        assert!(KeyMapConfig::default().problems().is_empty());
    }

    #[test]
    fn note_names() {
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(21), "A0");
        assert_eq!(note_name(108), "C8");
    }
}
