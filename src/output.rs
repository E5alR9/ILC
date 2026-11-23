//! OS keyboard injection and MIDI port helpers.

use enigo::{Direction, Enigo, Keyboard, Settings};

use crate::keymap::{Key, KeyEvent};

/// Something that can receive key events (real keyboard, or a test recorder).
pub trait KeySink: Send {
    fn send(&mut self, events: &[KeyEvent]) -> Result<(), String>;
}

/// Synthesises key presses with `enigo` (SendInput with scan codes on
/// Windows, CGEvent on macOS, XTest on Linux/X11).
pub struct SystemKeyboard {
    enigo: Enigo,
}

impl SystemKeyboard {
    pub fn new() -> Result<Self, String> {
        let settings = Settings { linux_delay: 0, release_keys_when_dropped: true, ..Settings::default() };
        Enigo::new(&settings).map(|enigo| Self { enigo }).map_err(|e| format!("Cannot simulate keyboard input: {e}"))
    }
}

impl KeySink for SystemKeyboard {
    fn send(&mut self, events: &[KeyEvent]) -> Result<(), String> {
        let mut first_err = None;
        for ev in events {
            let (key, dir) = match *ev {
                KeyEvent::Down(k) => (k, Direction::Press),
                KeyEvent::Up(k) => (k, Direction::Release),
            };
            let key = match key {
                Key::Char(c) => enigo::Key::Unicode(c),
                Key::Shift => enigo::Key::Shift,
                Key::Ctrl => enigo::Key::Control,
                Key::Alt => enigo::Key::Alt,
                Key::Space => enigo::Key::Space,
            };
            if let Err(e) = self.enigo.key(key, dir)
                && first_err.is_none()
            {
                first_err = Some(format!("Key {ev:?} failed: {e}"));
            }
        }
        first_err.map_or(Ok(()), Err)
    }
}

/// Names of the available MIDI output ports.
pub fn midi_output_ports() -> Result<Vec<String>, String> {
    let out = midir::MidiOutput::new("nanoMIDI").map_err(|e| e.to_string())?;
    Ok(out.ports().iter().filter_map(|p| out.port_name(p).ok()).collect())
}

/// Names of the available MIDI input ports.
pub fn midi_input_ports() -> Result<Vec<String>, String> {
    let inp = midir::MidiInput::new("nanoMIDI").map_err(|e| e.to_string())?;
    Ok(inp.ports().iter().filter_map(|p| inp.port_name(p).ok()).collect())
}

pub fn connect_midi_output(name: &str) -> Result<midir::MidiOutputConnection, String> {
    let out = midir::MidiOutput::new("nanoMIDI").map_err(|e| e.to_string())?;
    let port = out
        .ports()
        .into_iter()
        .find(|p| out.port_name(p).is_ok_and(|n| n == name))
        .ok_or_else(|| format!("MIDI output \"{name}\" not found"))?;
    out.connect(&port, "nanoMIDI playback").map_err(|e| e.to_string())
}
