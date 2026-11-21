//! Lock-freier Zustand zwischen Audio-Thread (MIDI rein), Art-Net-Thread
//! (Senden + Konfig neu laden) und GUI.
//!
//! Der Audio-Thread schreibt nur Atomics – keine Locks, keine Allokationen.

use std::sync::atomic::{
    AtomicBool, AtomicI32, AtomicU16, AtomicU32, AtomicU64, AtomicU8, Ordering::Relaxed,
};
use std::sync::{Arc, Mutex};

use crate::config::{Show, BLOCK_SIZE, DMX_SIZE};

/// Maximal 16 verschiedene Universen (= 16 MIDI-Kanäle).
pub const MAX_BUFS: usize = 16;

pub const EV_NOTE_ON: u32 = 1;
pub const EV_NOTE_OFF: u32 = 2;
pub const EV_PRESSURE: u32 = 3;
pub const EV_CC: u32 = 4;
pub const EV_RELEASE: u32 = 5;

type Row<T> = [T; DMX_SIZE];

fn row<T>(f: impl Fn() -> T) -> Row<T> {
    std::array::from_fn(|_| f())
}

pub struct Shared {
    /// aktueller Rohwert pro Slot (vor Master)
    pub values: [Row<AtomicU8>; MAX_BUFS],
    /// Wert ohne aktive Note
    pub rest: [Row<AtomicU8>; MAX_BUFS],
    /// wirkt Master/Blackout auf diesen Slot?
    pub intensity: [Row<AtomicBool>; MAX_BUFS],
    pub buf_universe: [AtomicU16; MAX_BUFS],
    pub buf_active: [AtomicBool; MAX_BUFS],
    /// pro MIDI-Kanal: -1 = aus, sonst (Puffer << 2) | Block
    pub midi_map: [AtomicI32; 16],
    pub cc_enabled: AtomicBool,

    pub reload_request: AtomicBool,
    pub packets_sent: AtomicU64,
    pub send_errors: AtomicU64,
    /// (Art << 24) | (Kanal << 16) | (Note << 8) | Wert
    pub last_event: AtomicU32,
    pub event_count: AtomicU32,

    /// nur GUI / Art-Net-Thread, nie Audio-Thread
    pub show: Mutex<Option<Arc<Show>>>,
    pub net_error: Mutex<Option<String>>,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            values: std::array::from_fn(|_| row(|| AtomicU8::new(0))),
            rest: std::array::from_fn(|_| row(|| AtomicU8::new(0))),
            intensity: std::array::from_fn(|_| row(|| AtomicBool::new(false))),
            buf_universe: std::array::from_fn(|_| AtomicU16::new(0)),
            buf_active: std::array::from_fn(|_| AtomicBool::new(false)),
            midi_map: std::array::from_fn(|_| AtomicI32::new(-1)),
            cc_enabled: AtomicBool::new(true),
            reload_request: AtomicBool::new(true),
            packets_sent: AtomicU64::new(0),
            send_errors: AtomicU64::new(0),
            last_event: AtomicU32::new(0),
            event_count: AtomicU32::new(0),
            show: Mutex::new(None),
            net_error: Mutex::new(None),
        }
    }
}

#[inline]
pub fn to_dmx(x: f32) -> u8 {
    (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

impl Shared {
    // ---------------- Audio-Thread ----------------

    #[inline]
    fn slot(&self, ch: u8, n: u8) -> Option<(usize, usize)> {
        if n as usize >= BLOCK_SIZE {
            return None;
        }
        let m = self.midi_map[(ch & 0x0F) as usize].load(Relaxed);
        if m < 0 {
            return None;
        }
        let buf = (m >> 2) as usize;
        let block = (m & 3) as usize;
        (buf < MAX_BUFS).then_some((buf, block * BLOCK_SIZE + n as usize))
    }

    #[inline]
    fn set(&self, ch: u8, n: u8, v: u8) {
        if let Some((b, a)) = self.slot(ch, n) {
            self.values[b][a].store(v, Relaxed);
        }
    }

    #[inline]
    fn log(&self, kind: u32, ch: u8, n: u8, v: u8) {
        self.last_event.store(
            (kind << 24) | ((ch as u32 & 0x0F) << 16) | ((n as u32) << 8) | v as u32,
            Relaxed,
        );
        self.event_count.fetch_add(1, Relaxed);
    }

    /// Velocity 1–127 → DMX 2–255
    pub fn note_on(&self, ch: u8, n: u8, velocity: f32) {
        if velocity <= 0.0 {
            return self.note_off(ch, n);
        }
        let v = to_dmx(velocity);
        self.set(ch, n, v);
        self.log(EV_NOTE_ON, ch, n, v);
    }

    /// Note-Off → Ruhewert aus der Fixture-Datei
    pub fn note_off(&self, ch: u8, n: u8) {
        let mut r = 0;
        if let Some((b, a)) = self.slot(ch, n) {
            r = self.rest[b][a].load(Relaxed);
            self.values[b][a].store(r, Relaxed);
        }
        self.log(EV_NOTE_OFF, ch, n, r);
    }

    /// Poly-Aftertouch: Wert einer gehaltenen Note live ändern
    pub fn pressure(&self, ch: u8, n: u8, pressure: f32) {
        let v = to_dmx(pressure);
        self.set(ch, n, v);
        self.log(EV_PRESSURE, ch, n, v);
    }

    pub fn cc(&self, ch: u8, cc: u8, value: f32) {
        match cc {
            // All Sound Off / All Notes Off (kommt z. B. beim Stop)
            120 | 123 => self.release_channel(ch),
            // übrige Channel-Mode-Messages ignorieren
            121..=127 => {}
            _ if self.cc_enabled.load(Relaxed) => {
                let v = to_dmx(value);
                self.set(ch, cc, v);
                self.log(EV_CC, ch, cc, v);
            }
            _ => {}
        }
    }

    /// alle 128 Slots dieses MIDI-Kanals auf Ruhewert
    pub fn release_channel(&self, ch: u8) {
        let m = self.midi_map[(ch & 0x0F) as usize].load(Relaxed);
        if m >= 0 {
            let b = (m >> 2) as usize;
            let start = (m & 3) as usize * BLOCK_SIZE;
            if b < MAX_BUFS {
                for a in start..start + BLOCK_SIZE {
                    self.values[b][a].store(self.rest[b][a].load(Relaxed), Relaxed);
                }
            }
        }
        self.log(EV_RELEASE, ch, 0, 0);
    }

    // ---------------- Art-Net-Thread / GUI ----------------

    /// Ausgabewert inkl. Master (0..1) auf Intensitätskanälen
    #[inline]
    pub fn output_value(&self, b: usize, a: usize, master: f32) -> u8 {
        let v = self.values[b][a].load(Relaxed);
        if self.intensity[b][a].load(Relaxed) {
            (v as f32 * master + 0.5) as u8
        } else {
            v
        }
    }

    pub fn raw_value(&self, b: usize, a: usize) -> u8 {
        self.values[b][a].load(Relaxed)
    }

    /// Fader in der GUI (gilt bis zum nächsten MIDI-Event auf dem Slot)
    pub fn set_manual(&self, b: usize, a: usize, v: u8) {
        if b < MAX_BUFS && a < DMX_SIZE {
            self.values[b][a].store(v, Relaxed);
        }
    }

    pub fn reset_all_to_rest(&self) {
        for b in 0..MAX_BUFS {
            if self.buf_active[b].load(Relaxed) {
                for a in 0..DMX_SIZE {
                    self.values[b][a].store(self.rest[b][a].load(Relaxed), Relaxed);
                }
            }
        }
    }

    pub fn buffer_for_universe(&self, u: u16) -> Option<usize> {
        (0..MAX_BUFS).find(|&b| {
            self.buf_active[b].load(Relaxed) && self.buf_universe[b].load(Relaxed) == u
        })
    }

    /// Neue Konfiguration übernehmen. Alle Slots gehen auf ihren Ruhewert.
    pub fn apply_show(&self, show: &Show) {
        for m in &self.midi_map {
            m.store(-1, Relaxed);
        }

        let mut unis: Vec<u16> = Vec::new();
        for t in show.midi.iter().flatten() {
            if !unis.contains(&t.universe) {
                unis.push(t.universe);
            }
        }

        for b in 0..MAX_BUFS {
            match unis.get(b) {
                Some(&u) => {
                    self.buf_universe[b].store(u, Relaxed);
                    for a in 0..DMX_SIZE {
                        let (rest, inten) = show.slot_info(u, a);
                        self.rest[b][a].store(rest, Relaxed);
                        self.intensity[b][a].store(inten, Relaxed);
                        self.values[b][a].store(rest, Relaxed);
                    }
                    self.buf_active[b].store(true, Relaxed);
                }
                None => self.buf_active[b].store(false, Relaxed),
            }
        }

        for (ch, t) in show.midi.iter().enumerate() {
            if let Some(t) = t {
                if let Some(b) = unis.iter().position(|&u| u == t.universe) {
                    self.midi_map[ch].store(((b as i32) << 2) | t.block as i32, Relaxed);
                }
            }
        }
        self.cc_enabled.store(show.cc_input, Relaxed);
    }

    pub fn set_net_error(&self, e: Option<String>) {
        if let Ok(mut g) = self.net_error.lock() {
            *g = e;
        }
    }
}
