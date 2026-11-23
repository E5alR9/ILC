//! Turns MIDI note events into keyboard events, tracking which key is held by
//! which note so chords, re‑triggers, sustain and finger limits behave.

use std::collections::HashMap;

use crate::keymap::{Key, KeyEvent, Layout, Modifier};

/// Behaviour switches for a [`Mapper`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapperOptions {
    /// Hold Space while the sustain pedal (CC64) is down.
    pub sustain: bool,
    /// Pedal counts as down when its value is above this.
    pub sustain_cutoff: u8,
    /// Re‑press a key when a second note wants a key that is already down.
    pub no_doubles: bool,
    /// Send Alt + velocity key before every note.
    pub velocity: bool,
    /// Semitones added to every note.
    pub transpose: i8,
    /// Max keys held at once (0 = unlimited).
    pub finger_limit: u8,
}

impl Default for MapperOptions {
    fn default() -> Self {
        Self { sustain: false, sustain_cutoff: 63, no_doubles: true, velocity: false, transpose: 0, finger_limit: 0 }
    }
}

/// What happened to a note‑on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteResult {
    /// Played as this (transposed) note.
    Played(u8),
    /// The (transposed) note has no key in the layout.
    OutOfRange(i16),
    /// Dropped because of the finger limit.
    Limited,
}

pub struct Mapper {
    pub layout: Layout,
    pub opts: MapperOptions,
    /// Key → note currently owning it.
    owner: HashMap<char, u8>,
    /// Played note → key it holds.
    note_key: [Option<char>; 128],
    /// (channel, source note) → played note, so note‑off finds the same key even
    /// if transpose changed in between.
    routed: [[Option<u8>; 128]; 16],
    held: usize,
    sustain_down: bool,
}

impl Mapper {
    pub fn new(layout: Layout, opts: MapperOptions) -> Self {
        Self {
            layout,
            opts,
            owner: HashMap::new(),
            note_key: [None; 128],
            routed: [[None; 128]; 16],
            held: 0,
            sustain_down: false,
        }
    }

    #[cfg(test)]
    pub fn held_keys(&self) -> usize {
        self.held
    }

    /// Played notes that currently hold a key.
    pub fn active_notes(&self) -> impl Iterator<Item = u8> + '_ {
        (0u8..128).filter(|&n| self.note_key[n as usize].is_some())
    }

    pub fn note_on(&mut self, ch: u8, note: u8, vel: u8, extra_shift: i8, out: &mut Vec<KeyEvent>) -> NoteResult {
        let ch = (ch & 15) as usize;
        let target = i16::from(note) + i16::from(self.opts.transpose) + i16::from(extra_shift);
        let Some(played) = u8::try_from(target).ok().filter(|n| *n < 128) else {
            return NoteResult::OutOfRange(target);
        };
        let Some(stroke) = self.layout.get(played) else {
            return NoteResult::OutOfRange(target);
        };
        let key_busy = self.owner.contains_key(&stroke.base);
        if self.opts.finger_limit > 0 && !key_busy && self.held >= usize::from(self.opts.finger_limit) {
            return NoteResult::Limited;
        }

        if self.opts.velocity
            && let Some(vk) = self.layout.velocity_key(vel)
        {
            out.extend([
                KeyEvent::Down(Key::Alt),
                KeyEvent::Down(Key::Char(vk)),
                KeyEvent::Up(Key::Char(vk)),
                KeyEvent::Up(Key::Alt),
            ]);
        }

        if let Some(prev) = self.owner.remove(&stroke.base) {
            self.note_key[prev as usize] = None;
            self.held -= 1;
            if self.opts.no_doubles {
                out.push(KeyEvent::Up(Key::Char(stroke.base)));
            }
        }

        let base = Key::Char(stroke.base);
        match stroke.modifier {
            Modifier::None => out.push(KeyEvent::Down(base)),
            Modifier::Shift => out.extend([KeyEvent::Down(Key::Shift), KeyEvent::Down(base), KeyEvent::Up(Key::Shift)]),
            Modifier::Ctrl => out.extend([KeyEvent::Down(Key::Ctrl), KeyEvent::Down(base), KeyEvent::Up(Key::Ctrl)]),
        }

        self.owner.insert(stroke.base, played);
        self.note_key[played as usize] = Some(stroke.base);
        self.held += 1;
        self.routed[ch][note as usize] = Some(played);
        NoteResult::Played(played)
    }

    /// Returns the played note that was released, if any.
    pub fn note_off(&mut self, ch: u8, note: u8, out: &mut Vec<KeyEvent>) -> Option<u8> {
        let played = self.routed[(ch & 15) as usize][note as usize & 127].take()?;
        self.release_played(played, out).then_some(played)
    }

    /// Release whatever key `played` holds (used by fixed hold length timers).
    pub fn release_played(&mut self, played: u8, out: &mut Vec<KeyEvent>) -> bool {
        let Some(key) = self.note_key[played as usize & 127].take() else {
            return false;
        };
        if self.owner.get(&key) == Some(&played) {
            self.owner.remove(&key);
        }
        self.held = self.held.saturating_sub(1);
        out.push(KeyEvent::Up(Key::Char(key)));
        true
    }

    /// Sustain pedal (CC64).
    pub fn pedal(&mut self, value: u8, out: &mut Vec<KeyEvent>) {
        if !self.opts.sustain {
            return;
        }
        if value > self.opts.sustain_cutoff && !self.sustain_down {
            self.sustain_down = true;
            out.push(KeyEvent::Down(Key::Space));
        } else if value < self.opts.sustain_cutoff && self.sustain_down {
            self.sustain_down = false;
            out.push(KeyEvent::Up(Key::Space));
        }
    }

    /// Release every held key, Space, and modifiers.
    pub fn release_all(&mut self, out: &mut Vec<KeyEvent>) {
        let mut keys: Vec<char> = self.owner.drain().map(|(k, _)| k).collect();
        keys.sort_unstable();
        out.extend(keys.into_iter().map(|k| KeyEvent::Up(Key::Char(k))));
        if self.sustain_down {
            self.sustain_down = false;
            out.push(KeyEvent::Up(Key::Space));
        }
        out.extend([KeyEvent::Up(Key::Shift), KeyEvent::Up(Key::Ctrl), KeyEvent::Up(Key::Alt)]);
        self.note_key = [None; 128];
        self.routed = [[None; 128]; 16];
        self.held = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{KeyMapConfig, default_drum_pads};
    use KeyEvent::{Down, Up};

    fn piano(opts: MapperOptions) -> Mapper {
        Mapper::new(Layout::piano(&KeyMapConfig::default(), true), opts)
    }

    #[test]
    fn plain_and_shifted_notes() {
        let mut m = piano(MapperOptions::default());
        let mut out = Vec::new();
        assert_eq!(m.note_on(0, 60, 100, 0, &mut out), NoteResult::Played(60));
        assert_eq!(out, vec![Down(Key::Char('t'))]);
        out.clear();
        m.note_on(0, 61, 100, 0, &mut out);
        assert_eq!(out, vec![Up(Key::Char('t')), Down(Key::Shift), Down(Key::Char('t')), Up(Key::Shift)]);
        out.clear();
        // 60 and 61 share 't'; with no_doubles the key is re‑pressed.
        assert_eq!(m.held_keys(), 1);
        assert_eq!(m.note_off(0, 60, &mut out), None, "60 lost ownership of 't' to 61");
        assert_eq!(m.note_off(0, 61, &mut out), Some(61));
        assert_eq!(out, vec![Up(Key::Char('t'))]);
        assert_eq!(m.held_keys(), 0);
    }

    #[test]
    fn no_doubles_retriggers() {
        let mut m = piano(MapperOptions::default());
        let mut out = Vec::new();
        m.note_on(0, 60, 100, 0, &mut out);
        out.clear();
        m.note_on(1, 60, 100, 0, &mut out);
        assert_eq!(out, vec![Up(Key::Char('t')), Down(Key::Char('t'))]);

        let mut m = piano(MapperOptions { no_doubles: false, ..Default::default() });
        out.clear();
        m.note_on(0, 60, 100, 0, &mut out);
        out.clear();
        m.note_on(1, 60, 100, 0, &mut out);
        assert_eq!(out, vec![Down(Key::Char('t'))]);
    }

    #[test]
    fn ctrl_range_and_out_of_range() {
        let mut m = piano(MapperOptions::default());
        let mut out = Vec::new();
        m.note_on(0, 21, 100, 0, &mut out);
        assert_eq!(out, vec![Down(Key::Ctrl), Down(Key::Char('1')), Up(Key::Ctrl)]);
        assert_eq!(m.note_on(0, 20, 100, 0, &mut out), NoteResult::OutOfRange(20));
        let mut m61 = Mapper::new(Layout::piano(&KeyMapConfig::default(), false), MapperOptions::default());
        assert_eq!(m61.note_on(0, 30, 100, 0, &mut out), NoteResult::OutOfRange(30));
    }

    #[test]
    fn transpose_is_remembered_for_note_off() {
        let mut m = piano(MapperOptions { transpose: 12, ..Default::default() });
        let mut out = Vec::new();
        assert_eq!(m.note_on(0, 48, 100, 0, &mut out), NoteResult::Played(60));
        m.opts.transpose = 0;
        out.clear();
        assert_eq!(m.note_off(0, 48, &mut out), Some(60));
        assert_eq!(out, vec![Up(Key::Char('t'))]);
    }

    #[test]
    fn velocity_and_sustain() {
        let mut m = piano(MapperOptions { velocity: true, sustain: true, ..Default::default() });
        let mut out = Vec::new();
        m.note_on(0, 60, 127, 0, &mut out);
        assert_eq!(
            out,
            vec![Down(Key::Alt), Down(Key::Char('c')), Up(Key::Char('c')), Up(Key::Alt), Down(Key::Char('t'))]
        );
        out.clear();
        m.pedal(63, &mut out);
        assert!(out.is_empty(), "value equal to cutoff changes nothing");
        m.pedal(127, &mut out);
        m.pedal(100, &mut out);
        m.pedal(0, &mut out);
        assert_eq!(out, vec![Down(Key::Space), Up(Key::Space)]);
    }

    #[test]
    fn finger_limit() {
        let mut m = piano(MapperOptions { finger_limit: 2, ..Default::default() });
        let mut out = Vec::new();
        assert!(matches!(m.note_on(0, 60, 1, 0, &mut out), NoteResult::Played(_)));
        assert!(matches!(m.note_on(0, 62, 1, 0, &mut out), NoteResult::Played(_)));
        assert_eq!(m.note_on(0, 64, 1, 0, &mut out), NoteResult::Limited);
        // Re‑triggering an already held key is still allowed.
        assert!(matches!(m.note_on(1, 60, 1, 0, &mut out), NoteResult::Played(_)));
        m.note_off(0, 62, &mut out);
        assert!(matches!(m.note_on(0, 64, 1, 0, &mut out), NoteResult::Played(_)));
    }

    #[test]
    fn release_all_clears_everything() {
        let mut m = piano(MapperOptions { sustain: true, ..Default::default() });
        let mut out = Vec::new();
        m.note_on(0, 60, 1, 0, &mut out);
        m.note_on(0, 64, 1, 0, &mut out);
        m.pedal(127, &mut out);
        out.clear();
        m.release_all(&mut out);
        assert!(out.contains(&Up(Key::Char('t'))));
        assert!(out.contains(&Up(Key::Char('u'))));
        assert!(out.contains(&Up(Key::Space)));
        assert_eq!(m.held_keys(), 0);
        assert_eq!(m.active_notes().count(), 0);
    }

    #[test]
    fn drums_layout() {
        let mut m = Mapper::new(Layout::drums(&default_drum_pads()), MapperOptions::default());
        let mut out = Vec::new();
        assert_eq!(m.note_on(9, 38, 100, 0, &mut out), NoteResult::Played(38));
        assert_eq!(out, vec![Down(Key::Char('c'))]);
        assert_eq!(m.note_on(9, 41, 100, 0, &mut out), NoteResult::OutOfRange(41));
    }
}
