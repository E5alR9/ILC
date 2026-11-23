//! System‑wide hotkeys (work while the game window has focus).

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::config;
use crate::engine::Cmd;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Play,
    Pause,
    Stop,
    SpeedUp,
    SlowDown,
}

impl Action {
    fn cmd(self) -> Cmd {
        match self {
            Action::Play => Cmd::Play { delay: 0.0 },
            Action::Pause => Cmd::TogglePause,
            Action::Stop => Cmd::Stop,
            Action::SpeedUp => Cmd::NudgeSpeed(1),
            Action::SlowDown => Cmd::NudgeSpeed(-1),
        }
    }
}

pub struct Hotkeys {
    manager: Option<GlobalHotKeyManager>,
    registered: Vec<HotKey>,
    map: Arc<RwLock<HashMap<u32, Action>>>,
}

impl Hotkeys {
    /// Must be called on the main (UI) thread.
    pub fn new(tx: Sender<Cmd>, notify: Arc<dyn Fn() + Send + Sync>) -> (Self, Option<String>) {
        let map: Arc<RwLock<HashMap<u32, Action>>> = Arc::default();
        let (manager, err) = match GlobalHotKeyManager::new() {
            Ok(m) => (Some(m), None),
            Err(e) => (None, Some(format!("Global hotkeys unavailable: {e}"))),
        };
        let m2 = map.clone();
        GlobalHotKeyEvent::set_event_handler(Some(move |ev: GlobalHotKeyEvent| {
            if ev.state() != HotKeyState::Pressed {
                return;
            }
            let action = m2.read().ok().and_then(|m| m.get(&ev.id()).copied());
            if let Some(a) = action {
                let _ = tx.send(a.cmd());
                notify();
            }
        }));
        (Self { manager, registered: Vec::new(), map }, err)
    }

    /// (Re)register all hotkeys. Returns human‑readable problems.
    pub fn apply(&mut self, cfg: &config::Hotkeys) -> Vec<String> {
        let mut problems = Vec::new();
        let Some(manager) = &self.manager else {
            return problems;
        };
        let _ = manager.unregister_all(&self.registered);
        self.registered.clear();
        let mut map = HashMap::new();
        for (action, text) in [
            (Action::Play, &cfg.play),
            (Action::Pause, &cfg.pause),
            (Action::Stop, &cfg.stop),
            (Action::SpeedUp, &cfg.speed_up),
            (Action::SlowDown, &cfg.slow_down),
        ] {
            if text.trim().is_empty() {
                continue;
            }
            match HotKey::from_str(text.trim()) {
                Ok(hk) => match manager.register(hk) {
                    Ok(()) => {
                        map.insert(hk.id(), action);
                        self.registered.push(hk);
                    }
                    Err(e) => problems.push(format!("{action:?} ({text}): {e}")),
                },
                Err(e) => problems.push(format!("{action:?} ({text}): {e}")),
            }
        }
        if let Ok(mut m) = self.map.write() {
            *m = map;
        }
        problems
    }
}

/// Convert a key pressed in the window into a hotkey string, e.g. `"ctrl+F6"`.
pub fn hotkey_string(key: eframe::egui::Key, mods: eframe::egui::Modifiers) -> Option<String> {
    use eframe::egui::Key as K;
    let name = match key {
        K::Escape | K::Enter | K::Space | K::Tab | K::Backspace => return None,
        K::ArrowUp => "ArrowUp".to_string(),
        K::ArrowDown => "ArrowDown".to_string(),
        K::ArrowLeft => "ArrowLeft".to_string(),
        K::ArrowRight => "ArrowRight".to_string(),
        K::PageUp => "PageUp".to_string(),
        K::PageDown => "PageDown".to_string(),
        other => other.name().to_string(),
    };
    let mut s = String::new();
    if mods.ctrl {
        s.push_str("ctrl+");
    }
    if mods.shift {
        s.push_str("shift+");
    }
    if mods.alt {
        s.push_str("alt+");
    }
    s.push_str(&name);
    HotKey::from_str(&s).ok().map(|_| s)
}
