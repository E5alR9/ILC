//! Real‑time playback thread.
//!
//! The song clock is `base_song + elapsed_real × speed`, so speed changes,
//! pauses and seeks never accumulate drift: every event is scheduled against
//! its absolute time in the file.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::config::{Humanize, KeyOptions, OutputMode};
use crate::keymap::{KeyEvent, Layout, LayoutKind};
use crate::mapper::{Mapper, NoteResult};
use crate::output::{KeySink, SystemKeyboard, connect_midi_output};
use crate::song::{Msg, Song};

pub const MIN_SPEED: f32 = 0.1;
pub const MAX_SPEED: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PlayState {
    Stopped = 0,
    Countdown = 1,
    Playing = 2,
    Paused = 3,
}

/// 128‑bit set of notes, readable from the UI thread without locking.
#[derive(Default)]
pub struct NoteSet([AtomicU64; 2]);

impl NoteSet {
    pub fn store(&self, notes: impl Iterator<Item = u8>) {
        let mut bits = [0u64; 2];
        for n in notes {
            let n = n & 127;
            bits[(n / 64) as usize] |= 1 << (n % 64);
        }
        self.0[0].store(bits[0], Ordering::Relaxed);
        self.0[1].store(bits[1], Ordering::Relaxed);
    }
    pub fn clear(&self) {
        self.0[0].store(0, Ordering::Relaxed);
        self.0[1].store(0, Ordering::Relaxed);
    }
    pub fn bits(&self) -> u128 {
        u128::from(self.0[0].load(Ordering::Relaxed)) | (u128::from(self.0[1].load(Ordering::Relaxed)) << 64)
    }
}

/// Bounded log shared with the UI.
#[derive(Default)]
pub struct Log(Mutex<VecDeque<(bool, String)>>);

impl Log {
    pub const MAX: usize = 300;
    pub fn push(&self, msg: impl Into<String>) {
        self.add(false, msg.into());
    }
    pub fn warn(&self, msg: impl Into<String>) {
        self.add(true, msg.into());
    }
    fn add(&self, warn: bool, msg: String) {
        if let Ok(mut q) = self.0.lock() {
            if q.len() >= Self::MAX {
                q.pop_front();
            }
            q.push_back((warn, msg));
        }
    }
    /// (is_warning, text) pairs, oldest first.
    pub fn lines(&self) -> Vec<(bool, String)> {
        self.0.lock().map(|q| q.iter().cloned().collect()).unwrap_or_default()
    }
    pub fn clear(&self) {
        if let Ok(mut q) = self.0.lock() {
            q.clear();
        }
    }
}

/// State the engine publishes for the UI.
pub struct Shared {
    state: AtomicU8,
    position: AtomicU64,
    speed: AtomicU32,
    countdown: AtomicU32,
    pub out_of_range: AtomicU64,
    pub active: NoteSet,
    pub log: Log,
}

impl Shared {
    fn new(speed: f32) -> Self {
        Self {
            state: AtomicU8::new(0),
            position: AtomicU64::new(0f64.to_bits()),
            speed: AtomicU32::new(speed.to_bits()),
            countdown: AtomicU32::new(0),
            out_of_range: AtomicU64::new(0),
            active: NoteSet::default(),
            log: Log::default(),
        }
    }
    pub fn state(&self) -> PlayState {
        match self.state.load(Ordering::Relaxed) {
            1 => PlayState::Countdown,
            2 => PlayState::Playing,
            3 => PlayState::Paused,
            _ => PlayState::Stopped,
        }
    }
    pub fn position(&self) -> f64 {
        f64::from_bits(self.position.load(Ordering::Relaxed))
    }
    pub fn speed(&self) -> f32 {
        f32::from_bits(self.speed.load(Ordering::Relaxed))
    }
    /// Seconds left before a delayed start.
    pub fn countdown(&self) -> f32 {
        f32::from_bits(self.countdown.load(Ordering::Relaxed))
    }
}

/// Everything the engine needs from the settings.
#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub mode: OutputMode,
    pub midi_port: String,
    pub piano: Layout,
    pub drums: Layout,
    pub keys: KeyOptions,
    pub humanize: Humanize,
    pub loop_song: bool,
    pub release_on_pause: bool,
    pub speed_step: f32,
}

pub enum Cmd {
    Load(Arc<Song>),
    /// Start / resume. `delay` seconds of countdown when starting from stop.
    Play {
        delay: f32,
    },
    TogglePause,
    Stop,
    Seek(f64),
    SetSpeed(f32),
    /// +1 / −1 × `speed_step`.
    NudgeSpeed(i8),
    Configure(Box<EngineConfig>),
    SetMutedTracks(Vec<bool>),
    Shutdown,
}

/// Handle owned by the UI.
/// Creates the key sink (the real keyboard, or a recorder in tests).
pub type SinkFactory = Arc<dyn Fn() -> Result<Box<dyn KeySink>, String> + Send + Sync>;

pub struct EngineHandle {
    pub tx: Sender<Cmd>,
    pub shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl EngineHandle {
    pub fn spawn(cfg: EngineConfig, speed: f32, notify: Box<dyn Fn() + Send>) -> Self {
        let factory: SinkFactory = Arc::new(|| SystemKeyboard::new().map(|k| Box::new(k) as Box<dyn KeySink>));
        Self::spawn_with(cfg, speed, notify, factory)
    }

    pub fn spawn_with(cfg: EngineConfig, speed: f32, notify: Box<dyn Fn() + Send>, factory: SinkFactory) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let shared = Arc::new(Shared::new(speed));
        let s2 = shared.clone();
        let thread = std::thread::Builder::new()
            .name("playback".into())
            .spawn(move || Engine::new(rx, s2, cfg, speed, notify, factory).run())
            .ok();
        Self { tx, shared, thread }
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

enum Output {
    Keys { mapper: Box<Mapper>, kind: LayoutKind, sink: Option<Box<dyn KeySink>>, sink_failed: bool },
    Midi { conn: Option<midir::MidiOutputConnection>, routed: Box<[[Option<u8>; 128]; 16]> },
}

struct Engine {
    rx: Receiver<Cmd>,
    shared: Arc<Shared>,
    notify: Box<dyn Fn() + Send>,
    factory: SinkFactory,
    cfg: EngineConfig,
    song: Option<Arc<Song>>,
    muted: Vec<bool>,
    out: Output,
    state: PlayState,
    idx: usize,
    position: f64,
    base_song: f64,
    base_instant: Instant,
    speed: f64,
    countdown_until: Instant,
    /// Fixed‑hold releases: (when, played note).
    holds: Vec<(Instant, u8)>,
    buf: Vec<KeyEvent>,
    _timer: Option<TimerResolution>,
}

impl Engine {
    fn new(
        rx: Receiver<Cmd>,
        shared: Arc<Shared>,
        cfg: EngineConfig,
        speed: f32,
        notify: Box<dyn Fn() + Send>,
        factory: SinkFactory,
    ) -> Self {
        let mut e = Self {
            rx,
            shared,
            notify,
            factory,
            out: Output::Midi { conn: None, routed: Box::new([[None; 128]; 16]) },
            cfg: cfg.clone(),
            song: None,
            muted: Vec::new(),
            state: PlayState::Stopped,
            idx: 0,
            position: 0.0,
            base_song: 0.0,
            base_instant: Instant::now(),
            speed: f64::from(speed.clamp(MIN_SPEED, MAX_SPEED)),
            countdown_until: Instant::now(),
            holds: Vec::new(),
            buf: Vec::with_capacity(64),
            _timer: None,
        };
        e.out = e.build_output();
        e
    }

    fn log(&self, msg: impl Into<String>) {
        self.shared.log.push(msg);
    }

    fn build_output(&self) -> Output {
        match self.cfg.mode {
            OutputMode::PianoKeys | OutputMode::DrumKeys => {
                let kind = if self.cfg.mode == OutputMode::DrumKeys { LayoutKind::Drums } else { LayoutKind::Piano };
                let layout = if kind == LayoutKind::Drums { self.cfg.drums.clone() } else { self.cfg.piano.clone() };
                Output::Keys {
                    mapper: Box::new(Mapper::new(layout, self.cfg.keys.mapper_options(kind))),
                    kind,
                    sink: None,
                    sink_failed: false,
                }
            }
            OutputMode::MidiOut => {
                let conn = if self.cfg.midi_port.is_empty() {
                    None
                } else {
                    match connect_midi_output(&self.cfg.midi_port) {
                        Ok(c) => {
                            self.log(format!("Connected to MIDI output \"{}\"", self.cfg.midi_port));
                            Some(c)
                        }
                        Err(e) => {
                            self.shared.log.warn(e);
                            None
                        }
                    }
                };
                Output::Midi { conn, routed: Box::new([[None; 128]; 16]) }
            }
        }
    }

    fn song_time(&self) -> f64 {
        self.base_song + self.base_instant.elapsed().as_secs_f64() * self.speed
    }

    fn rebase(&mut self, at: f64) {
        self.base_song = at;
        self.base_instant = Instant::now();
    }

    fn set_state(&mut self, s: PlayState) {
        if self.state != s {
            self.state = s;
            self.shared.state.store(s as u8, Ordering::Relaxed);
            // Ask the OS for 1 ms timer resolution only while playing.
            self._timer = matches!(s, PlayState::Playing | PlayState::Countdown).then(TimerResolution::new);
            (self.notify)();
        }
    }

    fn publish(&self) {
        self.shared.position.store(self.position.to_bits(), Ordering::Relaxed);
        match &self.out {
            Output::Keys { mapper, .. } => self.shared.active.store(mapper.active_notes()),
            Output::Midi { routed, .. } => self.shared.active.store(routed.iter().flatten().flatten().copied()),
        }
    }

    fn run(mut self) {
        loop {
            let deadline = self.next_deadline();
            let cmd = match deadline {
                None => match self.rx.recv() {
                    Ok(c) => Some(c),
                    Err(_) => return,
                },
                Some(due) => {
                    let now = Instant::now();
                    let rem = due.saturating_duration_since(now);
                    if rem > Duration::from_millis(2) {
                        // Sleep until ~1 ms before the event (max 50 ms so the
                        // position display keeps moving), then spin the rest.
                        let wait = (rem - Duration::from_millis(1)).min(Duration::from_millis(50));
                        match self.rx.recv_timeout(wait) {
                            Ok(c) => Some(c),
                            Err(RecvTimeoutError::Timeout) => None,
                            Err(RecvTimeoutError::Disconnected) => return,
                        }
                    } else {
                        match self.rx.try_recv() {
                            Ok(c) => Some(c),
                            Err(TryRecvError::Disconnected) => return,
                            Err(TryRecvError::Empty) => {
                                while Instant::now() < due {
                                    std::hint::spin_loop();
                                    std::thread::yield_now();
                                }
                                None
                            }
                        }
                    }
                }
            };
            if let Some(cmd) = cmd
                && !self.handle(cmd)
            {
                self.release_all();
                return;
            }
            self.tick();
        }
    }

    /// When the loop must wake up next (None = only on commands).
    fn next_deadline(&self) -> Option<Instant> {
        let hold = self.holds.iter().map(|h| h.0).min();
        let main = match self.state {
            PlayState::Stopped | PlayState::Paused => None,
            PlayState::Countdown => Some(self.countdown_until.min(Instant::now() + Duration::from_millis(100))),
            PlayState::Playing => {
                let next = self.song.as_ref().and_then(|s| s.events.get(self.idx)).map(|e| e.time);
                let now = Instant::now();
                Some(match next {
                    Some(t) => {
                        let ahead = ((t - self.song_time()) / self.speed).max(0.0);
                        now + Duration::from_secs_f64(ahead.min(0.05))
                    }
                    None => now,
                })
            }
        };
        match (main, hold) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Returns false on shutdown.
    fn handle(&mut self, cmd: Cmd) -> bool {
        match cmd {
            Cmd::Shutdown => return false,
            Cmd::Load(song) => {
                self.stop();
                self.muted = vec![false; song.tracks.len()];
                self.song = Some(song);
                self.shared.out_of_range.store(0, Ordering::Relaxed);
            }
            Cmd::Play { delay } => self.play(delay),
            Cmd::TogglePause => match self.state {
                PlayState::Playing | PlayState::Countdown => self.pause(),
                PlayState::Paused => self.play(0.0),
                PlayState::Stopped => {}
            },
            Cmd::Stop => {
                if self.state != PlayState::Stopped {
                    self.log("Stopped");
                }
                self.stop();
            }
            Cmd::Seek(t) => self.seek(t),
            Cmd::SetSpeed(s) => self.set_speed(s),
            Cmd::NudgeSpeed(dir) => {
                let s = self.speed as f32 + f32::from(dir) * self.cfg.speed_step;
                self.set_speed(s);
                self.log(format!("Speed {:.0}%", self.speed * 100.0));
            }
            Cmd::SetMutedTracks(m) => self.muted = m,
            Cmd::Configure(cfg) => self.configure(*cfg),
        }
        (self.notify)();
        true
    }

    fn configure(&mut self, cfg: EngineConfig) {
        let rebuild = cfg.mode != self.cfg.mode
            || (cfg.mode == OutputMode::MidiOut && cfg.midi_port != self.cfg.midi_port)
            || (cfg.mode == OutputMode::MidiOut && matches!(self.out, Output::Midi { conn: None, .. }));
        let layout_changed = match cfg.mode {
            OutputMode::PianoKeys => {
                cfg.piano.strokes != self.cfg.piano.strokes || cfg.piano.velocity != self.cfg.piano.velocity
            }
            OutputMode::DrumKeys => cfg.drums.strokes != self.cfg.drums.strokes,
            OutputMode::MidiOut => false,
        };
        if rebuild || layout_changed {
            self.release_all();
        }
        self.cfg = cfg;
        if rebuild {
            self.out = self.build_output();
        } else if let Output::Keys { mapper, kind, .. } = &mut self.out {
            mapper.opts = self.cfg.keys.mapper_options(*kind);
            if layout_changed {
                mapper.layout =
                    if *kind == LayoutKind::Drums { self.cfg.drums.clone() } else { self.cfg.piano.clone() };
            }
        }
        if !self.cfg.keys.fixed_hold && !self.holds.is_empty() {
            self.release_holds(true);
        }
    }

    fn set_speed(&mut self, s: f32) {
        let t = self.song_time();
        if self.state == PlayState::Playing {
            self.rebase(t);
        }
        self.speed = f64::from(s.clamp(MIN_SPEED, MAX_SPEED));
        self.shared.speed.store((self.speed as f32).to_bits(), Ordering::Relaxed);
    }

    fn play(&mut self, delay: f32) {
        let Some(song) = &self.song else {
            self.log("Pick a MIDI file first");
            return;
        };
        let title = song.title.clone();
        match self.state {
            PlayState::Playing | PlayState::Countdown => {}
            PlayState::Paused => {
                self.rebase(self.position);
                self.set_state(PlayState::Playing);
                self.log("Resumed");
            }
            PlayState::Stopped => {
                if delay > 0.0 {
                    self.countdown_until = Instant::now() + Duration::from_secs_f32(delay);
                    self.set_state(PlayState::Countdown);
                    self.log(format!("Starting \"{title}\" in {delay:.0} s, switch to your game now"));
                } else {
                    self.rebase(self.position);
                    self.set_state(PlayState::Playing);
                    self.log(format!("Playing \"{title}\""));
                }
            }
        }
    }

    fn pause(&mut self) {
        if self.state == PlayState::Playing {
            self.position = self.song_time();
        }
        if self.cfg.release_on_pause {
            self.release_all();
        }
        self.set_state(PlayState::Paused);
        self.shared.countdown.store(0, Ordering::Relaxed);
        self.log("Paused");
        self.publish();
    }

    fn stop(&mut self) {
        self.release_all();
        self.idx = 0;
        self.position = 0.0;
        self.shared.countdown.store(0, Ordering::Relaxed);
        self.set_state(PlayState::Stopped);
        self.publish();
    }

    fn seek(&mut self, t: f64) {
        let Some(song) = self.song.clone() else { return };
        let t = t.clamp(0.0, song.duration);
        self.release_all();
        self.idx = song.index_at(t);
        self.position = t;
        if self.state == PlayState::Playing {
            self.rebase(t);
        }
        // Restore instrument, controller and pitch‑bend state on MIDI devices.
        if let Output::Midi { conn: Some(conn), .. } = &mut self.out {
            let mut program = [None; 16];
            let mut bend = [None; 16];
            let mut ctrl: Vec<Option<u8>> = vec![None; 16 * 128];
            for e in &song.events[..self.idx] {
                match e.msg {
                    Msg::Program { ch, program: p } => program[ch as usize] = Some(p),
                    Msg::PitchBend { ch, value } => bend[ch as usize] = Some(value),
                    Msg::Control { ch, ctrl: c, value } if c != 64 && !(120..=127).contains(&c) => {
                        ctrl[ch as usize * 128 + c as usize] = Some(value);
                    }
                    _ => {}
                }
            }
            for ch in 0..16u8 {
                if let Some(p) = program[ch as usize] {
                    let (b, n) = Msg::Program { ch, program: p }.bytes();
                    let _ = conn.send(&b[..n]);
                }
                for c in 0..128u8 {
                    if let Some(value) = ctrl[ch as usize * 128 + c as usize] {
                        let (b, n) = Msg::Control { ch, ctrl: c, value }.bytes();
                        let _ = conn.send(&b[..n]);
                    }
                }
                if let Some(value) = bend[ch as usize] {
                    let (b, n) = Msg::PitchBend { ch, value }.bytes();
                    let _ = conn.send(&b[..n]);
                }
            }
        }
        self.publish();
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if !self.holds.is_empty() {
            self.release_holds(false);
        }
        match self.state {
            PlayState::Countdown => {
                let left = self.countdown_until.saturating_duration_since(now).as_secs_f32();
                self.shared.countdown.store(left.to_bits(), Ordering::Relaxed);
                if left <= 0.0 {
                    self.rebase(self.position);
                    self.set_state(PlayState::Playing);
                    if let Some(s) = &self.song {
                        self.log(format!("Playing \"{}\"", s.title));
                    }
                }
                (self.notify)();
            }
            PlayState::Playing => {
                let Some(song) = self.song.clone() else {
                    self.stop();
                    return;
                };
                let mut t = self.song_time();
                let mut changed = false;
                while let Some(ev) = song.events.get(self.idx) {
                    if ev.time > t {
                        break;
                    }
                    self.dispatch(ev.track, ev.msg);
                    changed = true;
                    self.idx += 1;
                    // Human timing: stretch/squeeze the gap to the next event.
                    let h = &self.cfg.humanize;
                    if h.enabled
                        && h.timing_pct > 0.0
                        && let Some(next) = song.events.get(self.idx)
                        && next.time > ev.time
                        && fastrand::f32() * 100.0 < h.timing_pct
                    {
                        let factor = 0.5 + fastrand::f64();
                        self.base_song -= (next.time - ev.time) * (factor - 1.0);
                        t = self.song_time();
                    }
                }
                self.position = t.min(song.duration);
                if self.idx >= song.events.len() {
                    self.release_all();
                    if self.cfg.loop_song && song.duration > 0.0 {
                        self.idx = 0;
                        self.position = 0.0;
                        self.rebase(0.0);
                        self.log("Looping");
                    } else {
                        self.log(format!("Finished \"{}\"", song.title));
                        self.stop();
                        return;
                    }
                }
                if changed {
                    self.publish();
                } else {
                    self.shared.position.store(self.position.to_bits(), Ordering::Relaxed);
                }
            }
            PlayState::Stopped | PlayState::Paused => {}
        }
    }

    fn dispatch(&mut self, track: u16, msg: Msg) {
        let muted = self.muted.get(track as usize).copied().unwrap_or(false);
        let h = &self.cfg.humanize;
        let wrong = if h.enabled
            && h.wrong_note_pct > 0.0
            && matches!(msg, Msg::NoteOn { .. })
            && fastrand::f32() * 100.0 < h.wrong_note_pct
        {
            fastrand::i8(-12..=12)
        } else {
            0
        };
        match &mut self.out {
            Output::Keys { mapper, sink, sink_failed, .. } => {
                match msg {
                    Msg::NoteOn { ch, key, vel } if !muted => {
                        match mapper.note_on(ch, key, vel, wrong, &mut self.buf) {
                            NoteResult::Played(n) => {
                                if self.cfg.keys.fixed_hold {
                                    let ms = u64::from(self.cfg.keys.hold_ms);
                                    self.holds.push((Instant::now() + Duration::from_millis(ms), n));
                                }
                            }
                            NoteResult::OutOfRange(_) => {
                                self.shared.out_of_range.fetch_add(1, Ordering::Relaxed);
                            }
                            NoteResult::Limited => {}
                        }
                    }
                    Msg::NoteOff { ch, key } if !self.cfg.keys.fixed_hold => {
                        mapper.note_off(ch, key, &mut self.buf);
                    }
                    Msg::Control { ctrl: 64, value, .. } => mapper.pedal(value, &mut self.buf),
                    _ => {}
                }
                if !self.buf.is_empty() {
                    if sink.is_none() && !*sink_failed {
                        match (self.factory)() {
                            Ok(k) => *sink = Some(k),
                            Err(e) => {
                                *sink_failed = true;
                                self.shared.log.warn(e);
                            }
                        }
                    }
                    if let Some(s) = sink
                        && let Err(e) = s.send(&self.buf)
                    {
                        self.shared.log.warn(e);
                    }
                    self.buf.clear();
                }
            }
            Output::Midi { conn, routed } => {
                let out = match msg {
                    Msg::NoteOn { ch, key, vel } => {
                        if muted {
                            return;
                        }
                        let n = i16::from(key) + i16::from(self.cfg.keys.transpose()) + i16::from(wrong);
                        let Ok(n) = u8::try_from(n) else { return };
                        if n > 127 {
                            return;
                        }
                        let slot = &mut routed[ch as usize][key as usize];
                        if let Some(old) = slot.take() {
                            send(conn, Msg::NoteOff { ch, key: old });
                        }
                        *slot = Some(n);
                        Msg::NoteOn { ch, key: n, vel }
                    }
                    Msg::NoteOff { ch, key } => {
                        let Some(n) = routed[ch as usize][key as usize].take() else { return };
                        Msg::NoteOff { ch, key: n }
                    }
                    Msg::Control { ctrl: 64, .. } if !self.cfg.keys.sustain => return,
                    other => other,
                };
                send(conn, out);
            }
        }
    }

    fn release_holds(&mut self, all: bool) {
        let now = Instant::now();
        let Output::Keys { mapper, sink, .. } = &mut self.out else {
            self.holds.clear();
            return;
        };
        self.holds.retain(|&(when, note)| {
            if all || when <= now {
                mapper.release_played(note, &mut self.buf);
                false
            } else {
                true
            }
        });
        if !self.buf.is_empty() {
            if let Some(s) = sink {
                let _ = s.send(&self.buf);
            }
            self.buf.clear();
            self.publish();
        }
    }

    fn release_all(&mut self) {
        self.holds.clear();
        match &mut self.out {
            Output::Keys { mapper, sink, .. } => {
                mapper.release_all(&mut self.buf);
                if let Some(s) = sink {
                    let _ = s.send(&self.buf);
                }
                self.buf.clear();
            }
            Output::Midi { conn, routed } => {
                for (ch, notes) in routed.iter_mut().enumerate() {
                    let ch = ch as u8;
                    for slot in notes.iter_mut() {
                        if let Some(n) = slot.take() {
                            send(conn, Msg::NoteOff { ch, key: n });
                        }
                    }
                }
                if conn.is_some() {
                    for ch in 0..16 {
                        send(conn, Msg::Control { ch, ctrl: 64, value: 0 }); // sustain off
                        send(conn, Msg::Control { ch, ctrl: 123, value: 0 }); // all notes off
                    }
                }
            }
        }
        self.shared.active.clear();
    }
}

fn send(conn: &mut Option<midir::MidiOutputConnection>, msg: Msg) {
    if let Some(c) = conn {
        let (b, n) = msg.bytes();
        let _ = c.send(&b[..n]);
    }
}

/// Raises the Windows timer resolution to 1 ms while alive (no‑op elsewhere).
struct TimerResolution;

impl TimerResolution {
    fn new() -> Self {
        #[cfg(windows)]
        // SAFETY: plain Win32 call with a constant argument; paired with timeEndPeriod in Drop.
        unsafe {
            windows_sys::Win32::Media::timeBeginPeriod(1);
        }
        Self
    }
}

impl Drop for TimerResolution {
    fn drop(&mut self) {
        #[cfg(windows)]
        // SAFETY: matches the timeBeginPeriod(1) in `new`.
        unsafe {
            windows_sys::Win32::Media::timeEndPeriod(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PlayerConfig;
    use crate::keymap::{Key, KeyMapConfig, default_drum_pads};

    type Recorded = Arc<Mutex<Vec<(Instant, KeyEvent)>>>;

    struct Recorder(Recorded);
    impl KeySink for Recorder {
        fn send(&mut self, events: &[KeyEvent]) -> Result<(), String> {
            let now = Instant::now();
            if let Ok(mut v) = self.0.lock() {
                v.extend(events.iter().map(|e| (now, *e)));
            }
            Ok(())
        }
    }

    fn cfg() -> EngineConfig {
        let p = PlayerConfig::default();
        EngineConfig {
            mode: OutputMode::PianoKeys,
            midi_port: String::new(),
            piano: Layout::piano(&KeyMapConfig::default(), true),
            drums: Layout::drums(&default_drum_pads()),
            keys: p.keys,
            humanize: p.humanize,
            loop_song: false,
            release_on_pause: true,
            speed_step: 0.05,
        }
    }

    fn engine(speed: f32) -> (EngineHandle, Recorded) {
        let rec: Recorded = Arc::default();
        let r2 = rec.clone();
        let factory: SinkFactory = Arc::new(move || Ok(Box::new(Recorder(r2.clone())) as Box<dyn KeySink>));
        (EngineHandle::spawn_with(cfg(), speed, Box::new(|| {}), factory), rec)
    }

    fn wait_for(h: &EngineHandle, s: PlayState, max: Duration) {
        let t = Instant::now();
        while h.shared.state() != s && t.elapsed() < max {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(h.shared.state(), s);
    }

    #[test]
    fn plays_on_time_at_double_speed() {
        let song = Arc::new(Song::parse(&crate::song::tests::test_midi()).expect("valid"));
        let (h, rec) = engine(2.0);
        h.send(Cmd::Load(song));
        let start = Instant::now();
        h.send(Cmd::Play { delay: 0.0 });
        wait_for(&h, PlayState::Playing, Duration::from_millis(200));
        wait_for(&h, PlayState::Stopped, Duration::from_secs(3));
        let events = rec.lock().expect("lock").clone();
        let at = |ev: KeyEvent| {
            events.iter().find(|(_, e)| *e == ev).map(|(t, _)| t.duration_since(start).as_secs_f64()).expect("event")
        };
        // Song times 0, 0.5, 0.5, 1.5 s → 0, 0.25, 0.25, 0.75 s at 2×.
        let t_on = at(KeyEvent::Down(Key::Char('t')));
        let t_off = at(KeyEvent::Up(Key::Char('t')));
        let u_on = at(KeyEvent::Down(Key::Char('u')));
        let u_off = events
            .iter()
            .filter(|(_, e)| *e == KeyEvent::Up(Key::Char('u')))
            .map(|(t, _)| t.duration_since(start).as_secs_f64())
            .next()
            .expect("u up");
        for (got, want) in [(t_on, 0.0), (t_off, 0.25), (u_on, 0.25), (u_off, 0.75)] {
            assert!((got - want).abs() < 0.02, "event at {got:.4}s, expected {want:.4}s");
        }
    }

    #[test]
    fn pause_seek_and_stop_release_keys() {
        let song = Arc::new(Song::parse(&crate::song::tests::test_midi()).expect("valid"));
        let (h, rec) = engine(1.0);
        h.send(Cmd::Load(song));
        h.send(Cmd::Play { delay: 0.0 });
        std::thread::sleep(Duration::from_millis(100));
        h.send(Cmd::TogglePause);
        wait_for(&h, PlayState::Paused, Duration::from_millis(200));
        let pos = h.shared.position();
        assert!((0.05..0.3).contains(&pos), "paused at {pos}");
        {
            let ev = rec.lock().expect("lock");
            assert!(ev.iter().any(|(_, e)| *e == KeyEvent::Up(Key::Char('t'))), "pause released held key");
        }
        h.send(Cmd::Seek(1.0));
        std::thread::sleep(Duration::from_millis(20));
        assert!((h.shared.position() - 1.0).abs() < 1e-9);
        h.send(Cmd::Stop);
        wait_for(&h, PlayState::Stopped, Duration::from_millis(200));
        assert_eq!(h.shared.position(), 0.0);
        assert_eq!(h.shared.active.bits(), 0);
    }

    #[test]
    fn speed_nudges_are_clamped() {
        let (h, _) = engine(4.98);
        h.send(Cmd::NudgeSpeed(1));
        h.send(Cmd::NudgeSpeed(1));
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(h.shared.speed(), MAX_SPEED);
        h.send(Cmd::SetSpeed(0.0));
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(h.shared.speed(), MIN_SPEED);
    }
}
