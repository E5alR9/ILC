//! Standard MIDI File loading: all tracks merged into one time‑ordered list of
//! channel events with absolute times in seconds (tempo map already applied).

use std::path::{Path, PathBuf};

use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};

/// A channel message, kept compact (8 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Msg {
    NoteOn { ch: u8, key: u8, vel: u8 },
    NoteOff { ch: u8, key: u8 },
    Control { ch: u8, ctrl: u8, value: u8 },
    Program { ch: u8, program: u8 },
    PitchBend { ch: u8, value: u16 },
    ChannelPressure { ch: u8, value: u8 },
    PolyPressure { ch: u8, key: u8, value: u8 },
}

impl Msg {
    /// Raw MIDI bytes.
    pub fn bytes(self) -> ([u8; 3], usize) {
        match self {
            Msg::NoteOn { ch, key, vel } => ([0x90 | ch, key, vel], 3),
            Msg::NoteOff { ch, key } => ([0x80 | ch, key, 0], 3),
            Msg::Control { ch, ctrl, value } => ([0xB0 | ch, ctrl, value], 3),
            Msg::Program { ch, program } => ([0xC0 | ch, program, 0], 2),
            Msg::PitchBend { ch, value } => ([0xE0 | ch, (value & 0x7F) as u8, (value >> 7) as u8 & 0x7F], 3),
            Msg::ChannelPressure { ch, value } => ([0xD0 | ch, value, 0], 2),
            Msg::PolyPressure { ch, key, value } => ([0xA0 | ch, key, value], 3),
        }
    }

    /// Parse raw bytes from a live MIDI input (running status not supported).
    pub fn parse(bytes: &[u8]) -> Option<Msg> {
        let status = *bytes.first()?;
        let ch = status & 0x0F;
        let a = bytes.get(1).copied().unwrap_or(0) & 0x7F;
        let b = bytes.get(2).copied().unwrap_or(0) & 0x7F;
        Some(match status & 0xF0 {
            0x80 => Msg::NoteOff { ch, key: a },
            0x90 if b == 0 => Msg::NoteOff { ch, key: a },
            0x90 => Msg::NoteOn { ch, key: a, vel: b },
            0xA0 => Msg::PolyPressure { ch, key: a, value: b },
            0xB0 => Msg::Control { ch, ctrl: a, value: b },
            0xC0 => Msg::Program { ch, program: a },
            0xD0 => Msg::ChannelPressure { ch, value: a },
            0xE0 => Msg::PitchBend { ch, value: u16::from(a) | (u16::from(b) << 7) },
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Event {
    /// Seconds from the start of the song at 100 % speed.
    pub time: f64,
    pub track: u16,
    pub msg: Msg,
}

#[derive(Clone, Debug, Default)]
pub struct TrackInfo {
    pub name: String,
    pub notes: usize,
    /// Notes on MIDI channel 10 (percussion).
    pub drum_notes: usize,
}

#[derive(Clone, Debug)]
pub struct Song {
    pub path: PathBuf,
    pub title: String,
    pub events: Vec<Event>,
    pub duration: f64,
    pub tracks: Vec<TrackInfo>,
    pub note_count: usize,
    pub lowest: u8,
    pub highest: u8,
    /// Note‑on count per note number.
    pub histogram: [u32; 128],
}

impl Song {
    pub fn load(path: &Path) -> Result<Song, String> {
        let data = std::fs::read(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
        let title = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let mut song = Song::parse(&data).map_err(|e| format!("{title}: {e}"))?;
        song.path = path.to_path_buf();
        song.title = title;
        Ok(song)
    }

    pub fn parse(data: &[u8]) -> Result<Song, String> {
        let smf = Smf::parse(data).map_err(|e| format!("not a valid MIDI file ({e})"))?;

        // Gather (tick, track, event) with absolute ticks.
        enum Raw {
            Tempo(u32),
            Msg(Msg),
        }
        let mut raw: Vec<(u64, u16, Raw)> = Vec::new();
        let mut tracks = Vec::with_capacity(smf.tracks.len());
        for (ti, track) in smf.tracks.iter().enumerate() {
            let ti = u16::try_from(ti).unwrap_or(u16::MAX);
            let mut tick: u64 = 0;
            let mut info = TrackInfo::default();
            for ev in track {
                tick += u64::from(ev.delta.as_int());
                match ev.kind {
                    TrackEventKind::Meta(MetaMessage::Tempo(t)) => raw.push((tick, ti, Raw::Tempo(t.as_int()))),
                    TrackEventKind::Meta(MetaMessage::TrackName(n)) if info.name.is_empty() => {
                        info.name = String::from_utf8_lossy(n).trim().to_string();
                    }
                    TrackEventKind::Meta(MetaMessage::InstrumentName(n)) if info.name.is_empty() => {
                        info.name = String::from_utf8_lossy(n).trim().to_string();
                    }
                    TrackEventKind::Midi { channel, message } => {
                        let ch = channel.as_int();
                        let msg = match message {
                            MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                                info.notes += 1;
                                if ch == 9 {
                                    info.drum_notes += 1;
                                }
                                Msg::NoteOn { ch, key: key.as_int(), vel: vel.as_int() }
                            }
                            MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                                Msg::NoteOff { ch, key: key.as_int() }
                            }
                            MidiMessage::Controller { controller, value } => {
                                Msg::Control { ch, ctrl: controller.as_int(), value: value.as_int() }
                            }
                            MidiMessage::ProgramChange { program } => Msg::Program { ch, program: program.as_int() },
                            MidiMessage::PitchBend { bend } => Msg::PitchBend { ch, value: bend.0.as_int() },
                            MidiMessage::ChannelAftertouch { vel } => Msg::ChannelPressure { ch, value: vel.as_int() },
                            MidiMessage::Aftertouch { key, vel } => {
                                Msg::PolyPressure { ch, key: key.as_int(), value: vel.as_int() }
                            }
                        };
                        raw.push((tick, ti, Raw::Msg(msg)));
                    }
                    _ => {}
                }
            }
            tracks.push(info);
        }
        // Stable: keeps per‑track order for events on the same tick.
        raw.sort_by_key(|(tick, _, _)| *tick);

        // Ticks → seconds.
        let to_secs: Box<dyn Fn(u64, u32) -> f64> = match smf.header.timing {
            Timing::Metrical(tpq) => {
                let tpq = f64::from(tpq.as_int().max(1));
                Box::new(move |ticks, tempo| ticks as f64 * f64::from(tempo) / 1_000_000.0 / tpq)
            }
            Timing::Timecode(fps, sub) => {
                let tps = f64::from(fps.as_f32()) * f64::from(sub.max(1));
                Box::new(move |ticks, _| ticks as f64 / tps)
            }
        };
        let mut tempo: u32 = 500_000;
        let mut last_tick = 0u64;
        let mut time = 0.0f64;
        let mut events = Vec::with_capacity(raw.len());
        let (mut lowest, mut highest, mut note_count) = (127u8, 0u8, 0usize);
        let mut histogram = [0u32; 128];
        for (tick, track, r) in raw {
            time += to_secs(tick - last_tick, tempo);
            last_tick = tick;
            match r {
                Raw::Tempo(t) => tempo = t.max(1),
                Raw::Msg(msg) => {
                    if let Msg::NoteOn { key, .. } = msg {
                        note_count += 1;
                        histogram[key as usize & 127] += 1;
                        lowest = lowest.min(key);
                        highest = highest.max(key);
                    }
                    events.push(Event { time, track, msg });
                }
            }
        }
        if note_count == 0 {
            (lowest, highest) = (0, 0);
        }
        let duration = events.last().map_or(0.0, |e| e.time);
        Ok(Song {
            path: PathBuf::new(),
            title: String::new(),
            events,
            duration,
            tracks,
            note_count,
            lowest,
            highest,
            histogram,
        })
    }

    /// How many notes would have a key with this transpose.
    pub fn playable(&self, layout: &crate::keymap::Layout, transpose: i16) -> u32 {
        (0..128i16)
            .filter(|&n| {
                let t = n + transpose;
                (0..128).contains(&t) && layout.get(t as u8).is_some()
            })
            .map(|n| self.histogram[n as usize])
            .sum()
    }

    /// Index of the first event at or after `t`.
    pub fn index_at(&self, t: f64) -> usize {
        self.events.partition_point(|e| e.time < t)
    }
}

/// `m:ss` (or `h:mm:ss`).
pub fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Build a tiny format‑1 file: track 0 tempo, track 1 notes.
    pub fn test_midi() -> Vec<u8> {
        let mut v = Vec::new();
        v.extend(b"MThd");
        v.extend(6u32.to_be_bytes());
        v.extend(1u16.to_be_bytes()); // format 1
        v.extend(2u16.to_be_bytes()); // tracks
        v.extend(96u16.to_be_bytes()); // ticks per quarter
        let track = |body: &[u8], v: &mut Vec<u8>| {
            v.extend(b"MTrk");
            v.extend((body.len() as u32).to_be_bytes());
            v.extend(body);
        };
        // 120 bpm for a quarter, then 60 bpm.
        track(
            &[
                0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, 0x60, 0xFF, 0x51, 0x03, 0x0F, 0x42, 0x40, 0x00, 0xFF, 0x2F,
                0x00,
            ],
            &mut v,
        );
        track(
            &[
                0x00, 0xFF, 0x03, 0x05, b'P', b'i', b'a', b'n', b'o', // name
                0x00, 0x90, 60, 100, // on at 0
                0x60, 0x80, 60, 0, // off at 1 quarter (0.5 s)
                0x00, 0x90, 64, 90, // on at 0.5 s
                0x60, 0x90, 64, 0, // off (vel 0) after 1 quarter at 60 bpm → 1.5 s
                0x00, 0xFF, 0x2F, 0x00,
            ],
            &mut v,
        );
        v
    }

    #[test]
    fn parses_tempo_map() {
        let s = Song::parse(&test_midi()).expect("valid");
        assert_eq!(s.note_count, 2);
        assert_eq!(s.tracks[1].name, "Piano");
        assert_eq!((s.lowest, s.highest), (60, 64));
        let times: Vec<f64> = s.events.iter().map(|e| e.time).collect();
        assert_eq!(times, vec![0.0, 0.5, 0.5, 1.5]);
        assert_eq!(s.events[3].msg, Msg::NoteOff { ch: 0, key: 64 });
        assert!((s.duration - 1.5).abs() < 1e-9);
        assert_eq!(s.index_at(0.5), 1);
        assert_eq!(s.index_at(0.6), 3);
    }

    #[test]
    fn rejects_garbage() {
        assert!(Song::parse(b"hello").is_err());
    }

    #[test]
    fn raw_roundtrip() {
        for m in [
            Msg::NoteOn { ch: 3, key: 60, vel: 1 },
            Msg::NoteOff { ch: 0, key: 1 },
            Msg::Control { ch: 15, ctrl: 64, value: 127 },
            Msg::Program { ch: 2, program: 5 },
            Msg::PitchBend { ch: 1, value: 8192 },
        ] {
            let (b, n) = m.bytes();
            assert_eq!(Msg::parse(&b[..n]), Some(m));
        }
        assert_eq!(fmt_time(65.0), "1:05");
        assert_eq!(fmt_time(3725.0), "1:02:05");
    }
}

/// `NANOMIDI_BENCH_FILE=path cargo test --release load_bench -- --ignored --nocapture`
#[cfg(test)]
#[test]
#[ignore = "benchmark; needs NANOMIDI_BENCH_FILE"]
fn load_bench() {
    let Ok(path) = std::env::var("NANOMIDI_BENCH_FILE") else { return };
    let start = std::time::Instant::now();
    let song = Song::load(Path::new(&path)).expect("load");
    println!("loaded {} notes / {} events in {:?}", song.note_count, song.events.len(), start.elapsed());
}
