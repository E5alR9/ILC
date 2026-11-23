//! Live MIDI keyboard → QWERTY keystrokes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::engine::{Log, NoteSet};
use crate::keymap::{KeyEvent, Layout};
use crate::mapper::{Mapper, MapperOptions, NoteResult};
use crate::output::{KeySink, SystemKeyboard};
use crate::song::Msg;

struct Core {
    mapper: Mapper,
    sink: Option<Box<dyn KeySink>>,
    fixed_hold: Option<Duration>,
    holds: Vec<(Instant, u8)>,
    buf: Vec<KeyEvent>,
}

impl Core {
    fn flush(&mut self, log: &Log) {
        if self.buf.is_empty() {
            return;
        }
        if let Some(s) = &mut self.sink
            && let Err(e) = s.send(&self.buf)
        {
            log.warn(e);
        }
        self.buf.clear();
    }

    fn handle(&mut self, msg: Msg, log: &Log, active: &NoteSet) {
        match msg {
            Msg::NoteOn { ch, key, vel } => match self.mapper.note_on(ch, key, vel, 0, &mut self.buf) {
                NoteResult::Played(n) => {
                    if let Some(d) = self.fixed_hold {
                        self.holds.push((Instant::now() + d, n));
                    }
                }
                NoteResult::OutOfRange(n) => log.push(format!("Note {n} is outside the layout")),
                NoteResult::Limited => {}
            },
            Msg::NoteOff { ch, key } if self.fixed_hold.is_none() => {
                self.mapper.note_off(ch, key, &mut self.buf);
            }
            Msg::Control { ctrl: 64, value, .. } => self.mapper.pedal(value, &mut self.buf),
            _ => {}
        }
        self.flush(log);
        active.store(self.mapper.active_notes());
    }

    fn release_due(&mut self, all: bool, log: &Log, active: &NoteSet) {
        let now = Instant::now();
        let mapper = &mut self.mapper;
        let buf = &mut self.buf;
        self.holds.retain(|&(when, n)| {
            if all || when <= now {
                mapper.release_played(n, buf);
                false
            } else {
                true
            }
        });
        if !self.buf.is_empty() {
            self.flush(log);
            active.store(self.mapper.active_notes());
        }
    }
}

/// An open live connection. Dropping it releases every key.
pub struct LiveInput {
    conn: Option<midir::MidiInputConnection<()>>,
    core: Arc<Mutex<Core>>,
    stop: Arc<AtomicBool>,
    pub active: Arc<NoteSet>,
    log: Arc<Log>,
    pub port: String,
}

impl LiveInput {
    pub fn connect(
        port_name: &str,
        layout: Layout,
        opts: MapperOptions,
        fixed_hold: Option<Duration>,
        log: Arc<Log>,
        notify: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<Self, String> {
        let input = midir::MidiInput::new("nanoMIDI live").map_err(|e| e.to_string())?;
        let port = input
            .ports()
            .into_iter()
            .find(|p| input.port_name(p).is_ok_and(|n| n == port_name))
            .ok_or_else(|| format!("MIDI input \"{port_name}\" not found"))?;
        let sink: Box<dyn KeySink> = Box::new(SystemKeyboard::new()?);
        let core = Arc::new(Mutex::new(Core {
            mapper: Mapper::new(layout, opts),
            sink: Some(sink),
            fixed_hold,
            holds: Vec::new(),
            buf: Vec::new(),
        }));
        let active = Arc::new(NoteSet::default());
        let (c2, a2, l2, n2) = (core.clone(), active.clone(), log.clone(), notify.clone());
        let conn = input
            .connect(
                &port,
                "nanoMIDI live input",
                move |_, bytes, ()| {
                    if let Some(msg) = Msg::parse(bytes)
                        && let Ok(mut core) = c2.lock()
                    {
                        core.handle(msg, &l2, &a2);
                        n2();
                    }
                },
                (),
            )
            .map_err(|e| e.to_string())?;

        // Timer thread for fixed hold length.
        let stop = Arc::new(AtomicBool::new(false));
        {
            let (core, stop, active, log) = (core.clone(), stop.clone(), active.clone(), log.clone());
            let _ = std::thread::Builder::new().name("live-holds".into()).spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(2));
                    if let Ok(mut c) = core.lock()
                        && !c.holds.is_empty()
                    {
                        c.release_due(false, &log, &active);
                        notify();
                    }
                }
            });
        }
        log.push(format!("Listening to \"{port_name}\""));
        Ok(Self { conn: Some(conn), core, stop, active, log, port: port_name.to_string() })
    }

    pub fn update(&self, layout: Layout, opts: MapperOptions, fixed_hold: Option<Duration>) {
        if let Ok(mut c) = self.core.lock() {
            let layout_changed =
                c.mapper.layout.strokes != layout.strokes || c.mapper.layout.velocity != layout.velocity;
            if layout_changed {
                c.release_due(true, &self.log, &self.active);
                let mut buf = std::mem::take(&mut c.buf);
                c.mapper.release_all(&mut buf);
                c.buf = buf;
                c.flush(&self.log);
                c.mapper.layout = layout;
            }
            c.mapper.opts = opts;
            if fixed_hold.is_none() {
                c.release_due(true, &self.log, &self.active);
            }
            c.fixed_hold = fixed_hold;
        }
    }
}

impl Drop for LiveInput {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(c) = self.conn.take() {
            c.close();
        }
        if let Ok(mut c) = self.core.lock() {
            let mut buf = std::mem::take(&mut c.buf);
            c.mapper.release_all(&mut buf);
            c.buf = buf;
            c.flush(&self.log);
        }
        self.active.clear();
        self.log.push("Live input closed");
    }
}
