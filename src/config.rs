//! Textdatei-basiertes Fixture-Management.
//!
//! Ordnerstruktur (Standard: Dokumente/DMX-Bridge):
//!   config.txt          Art-Net-Ziel, MIDI-Kanal → Universum/Block
//!   patch.txt           Name | Fixture | Universum | Adresse | [Modus] [anzahl=n]
//!   fixtures/*.txt      eigenes Textformat (ein Typ pro Datei)
//!   fixtures/*.qxf      QLC+-Fixture-Definitionen, direkt importiert
//!
//! Der Parser ist absichtlich tolerant: unbekannte Schlüssel und zusätzliche
//! Spalten werden ignoriert (Aufwärtskompatibilität), Fehler landen als
//! Meldung in der GUI, statt das Plugin abstürzen zu lassen.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use crate::i18n::t;
use crate::qxf;
use crate::tf;

pub const BLOCK_SIZE: usize = 128;
pub const DMX_SIZE: usize = 512;
pub const MAX_UNIVERSE: u16 = 0x7FFF;
/// Strobe-Bereich, wenn nichts angegeben ist (typische China-PARs: 1–25 Hz)
pub const DEFAULT_STROBE: (f32, f32) = (1.0, 25.0);

pub type Rgb = [u8; 3];

const EXAMPLES: &[(&str, &str)] = &[
    ("config.txt", include_str!("../example-config/config.txt")),
    ("patch.txt", include_str!("../example-config/patch.txt")),
    (
        "fixtures/uking_wash_8ch.txt",
        include_str!("../example-config/fixtures/uking_wash_8ch.txt"),
    ),
    (
        "fixtures/par_rgbwauv_8ch.txt",
        include_str!("../example-config/fixtures/par_rgbwauv_8ch.txt"),
    ),
    (
        "fixtures/rgbw_4ch.txt",
        include_str!("../example-config/fixtures/rgbw_4ch.txt"),
    ),
    (
        "fixtures/dimmer_1ch.txt",
        include_str!("../example-config/fixtures/dimmer_1ch.txt"),
    ),
    (
        "fixtures/example_movinghead.txt",
        include_str!("../example-config/fixtures/example_movinghead.txt"),
    ),
];

// ---------------------------------------------------------------------------
// Kanaltypen
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChanKind {
    Dimmer,
    DimmerFine,
    Red,
    Green,
    Blue,
    White,
    Amber,
    Uv,
    Lime,
    Indigo,
    Cyan,
    Magenta,
    Yellow,
    ColorWheel,
    ColorMacro,
    Cto,
    Shutter,
    Pan,
    PanFine,
    Tilt,
    TiltFine,
    PanTiltSpeed,
    Gobo,
    GoboRotation,
    Prism,
    PrismRotation,
    Focus,
    Zoom,
    Iris,
    Frost,
    Rotation,
    Effect,
    EffectSpeed,
    Fog,
    Maintenance,
    Other,
}

use ChanKind as K;

/// (type, German label, English label, keywords – first = German file keyword,
/// the English file keyword is listed separately)
const KIND_TABLE: &[(ChanKind, &str, &str, &str, &[&str])] = &[
    (K::Dimmer, "Dimmer", "Dimmer", "dimmer", &["dimmer", "master", "intensity", "intensität", "helligkeit"]),
    (K::DimmerFine, "Dimmer fein", "Dimmer fine", "dimmer_fine", &["dimmer_fein", "dimmer_fine"]),
    (K::Red, "Rot", "Red", "red", &["rot", "red", "r"]),
    (K::Green, "Grün", "Green", "green", &["grün", "gruen", "green", "g"]),
    (K::Blue, "Blau", "Blue", "blue", &["blau", "blue", "b"]),
    (K::White, "Weiß", "White", "white", &["weiß", "weiss", "white", "w"]),
    (K::Amber, "Amber", "Amber", "amber", &["amber", "a"]),
    (K::Uv, "UV", "UV", "uv", &["uv", "u"]),
    (K::Lime, "Lime", "Lime", "lime", &["lime"]),
    (K::Indigo, "Indigo", "Indigo", "indigo", &["indigo"]),
    (K::Cyan, "Cyan", "Cyan", "cyan", &["cyan", "c"]),
    (K::Magenta, "Magenta", "Magenta", "magenta", &["magenta", "m"]),
    (K::Yellow, "Gelb", "Yellow", "yellow", &["gelb", "yellow", "y"]),
    (K::ColorWheel, "Farbrad", "Color wheel", "colorwheel", &["farbrad", "colorwheel", "color_wheel"]),
    (K::ColorMacro, "Farbmakro", "Color macro", "colormacro", &["farbmakro", "colormacro", "color_macro"]),
    (K::Cto, "Farbtemperatur", "Color temp.", "cto", &["cto", "farbtemperatur", "ctc", "ctb"]),
    (K::Shutter, "Strobe/Shutter", "Strobe/Shutter", "strobe", &["strobe", "strobo", "shutter", "blitz"]),
    (K::Pan, "Pan", "Pan", "pan", &["pan"]),
    (K::PanFine, "Pan fein", "Pan fine", "pan_fine", &["pan_fein", "pan_fine"]),
    (K::Tilt, "Tilt", "Tilt", "tilt", &["tilt"]),
    (K::TiltFine, "Tilt fein", "Tilt fine", "tilt_fine", &["tilt_fein", "tilt_fine"]),
    (K::PanTiltSpeed, "Pan/Tilt-Speed", "Pan/tilt speed", "pt_speed", &["pt_speed", "pan_tilt_speed", "bewegung_speed"]),
    (K::Gobo, "Gobo", "Gobo", "gobo", &["gobo"]),
    (K::GoboRotation, "Gobo-Rotation", "Gobo rotation", "gobo_rotation", &["gobo_rotation", "goborotation"]),
    (K::Prism, "Prisma", "Prism", "prism", &["prisma", "prism"]),
    (K::PrismRotation, "Prisma-Rotation", "Prism rotation", "prism_rotation", &["prisma_rotation", "prism_rotation"]),
    (K::Focus, "Fokus", "Focus", "focus", &["fokus", "focus"]),
    (K::Zoom, "Zoom", "Zoom", "zoom", &["zoom"]),
    (K::Iris, "Iris", "Iris", "iris", &["iris"]),
    (K::Frost, "Frost", "Frost", "frost", &["frost"]),
    (K::Rotation, "Rotation", "Rotation", "rotation", &["rotation", "drehung"]),
    (K::Effect, "Effekt/Programm", "Effect/program", "effect", &["effekt", "effect", "programm", "program", "makro", "macro", "modus"]),
    (K::EffectSpeed, "Speed", "Speed", "speed", &["speed", "geschwindigkeit", "effekt_speed"]),
    (K::Fog, "Nebel", "Fog", "fog", &["nebel", "fog", "haze", "hazer"]),
    (K::Maintenance, "Wartung", "Maintenance", "maintenance", &["wartung", "maintenance", "reset"]),
    (K::Other, "Sonst", "Other", "other", &["sonst", "other", "sonstiges", "nichts", "-"]),
];

impl ChanKind {
    pub fn parse(s: &str) -> Option<Self> {
        let l = s.trim().to_lowercase();
        KIND_TABLE
            .iter()
            .find(|(_, _, _, _, keys)| keys.contains(&l.as_str()))
            .map(|(k, ..)| *k)
    }

    pub fn label(self) -> &'static str {
        KIND_TABLE
            .iter()
            .find(|(k, ..)| *k == self)
            .map(|(_, de, en, _, _)| t(de, en))
            .unwrap_or("?")
    }

    /// Keyword the editor writes into files (in the current UI language;
    /// both languages are always accepted when reading)
    pub fn keyword(self) -> &'static str {
        KIND_TABLE
            .iter()
            .find(|(k, ..)| *k == self)
            .map(|(_, _, _, en_key, keys)| t(keys[0], en_key))
            .unwrap_or("other")
    }

    pub fn all() -> impl Iterator<Item = ChanKind> {
        KIND_TABLE.iter().map(|(k, ..)| *k)
    }

    pub fn keyword_list() -> String {
        KIND_TABLE
            .iter()
            .map(|(_, _, _, en_key, keys)| t(keys[0], en_key))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Additive Lichtfarben (für Vorschau und Master ohne Dimmerkanal)
    pub fn is_additive(self) -> bool {
        matches!(
            self,
            K::Red | K::Green | K::Blue | K::White | K::Amber | K::Uv | K::Lime | K::Indigo
        )
    }
}

// ---------------------------------------------------------------------------
// Capabilities (Wertebereiche pro Kanal, wie bei QLC+)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// keine Funktion / nur Beschriftung
    Nothing,
    /// Shutter offen, Licht an (kein Strobe)
    Open,
    /// Shutter zu, dunkel
    Closed,
    /// Strobe von `from` bis `to` Hz über den Bereich
    Strobe { from: f32, to: f32, random: bool },
    /// Farbe auf Farbrad/Makro (zweite Farbe = geteiltes Rad)
    Color(Rgb, Option<Rgb>),
    /// freie Funktion (Gobo 3, Auto-Programm …)
    Text,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Capability {
    pub min: u8,
    pub max: u8,
    pub effect: Effect,
    pub label: String,
}

impl Capability {
    pub fn strobe_hz(&self, v: u8) -> Option<f32> {
        let Effect::Strobe { from, to, .. } = self.effect else {
            return None;
        };
        if self.max <= self.min {
            return Some(from);
        }
        let t = v.saturating_sub(self.min) as f32 / (self.max - self.min) as f32;
        Some(from + (to - from) * t.clamp(0.0, 1.0))
    }

    pub fn cap_contains(&self, v: u8) -> bool {
        v >= self.min && v <= self.max
    }

    /// DMX-Wert für eine Wunschfrequenz (None, wenn außerhalb des Bereichs)
    pub fn value_for_hz(&self, hz: f32) -> Option<u8> {
        let Effect::Strobe { from, to, .. } = self.effect else {
            return None;
        };
        let (lo, hi) = (from.min(to), from.max(to));
        if hz < lo - 1e-3 || hz > hi + 1e-3 {
            return None;
        }
        if (to - from).abs() < 1e-6 || self.max == self.min {
            return Some(self.min);
        }
        let t = ((hz - from) / (to - from)).clamp(0.0, 1.0);
        Some((self.min as f32 + t * (self.max - self.min) as f32).round() as u8)
    }

    pub fn describe(&self, v: u8) -> String {
        match self.strobe_hz(v) {
            Some(hz) => format!("{} · {hz:.1} Hz", self.label),
            None => self.label.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FixtureChannel {
    pub kind: ChanKind,
    pub label: String,
    pub rest: u8,
    pub caps: Vec<Capability>,
}

impl FixtureChannel {
    pub fn cap_at(&self, v: u8) -> Option<&Capability> {
        self.caps.iter().find(|c| v >= c.min && v <= c.max)
    }
}

/// Standard für Strobe-Kanäle ohne eigene Bereiche: 0–9 offen, 10–255 = 1–25 Hz
pub fn default_shutter_caps() -> Vec<Capability> {
    vec![
        Capability {
            min: 0,
            max: 9,
            effect: Effect::Open,
            label: t("offen", "open").into(),
        },
        Capability {
            min: 10,
            max: 255,
            effect: Effect::Strobe {
                from: DEFAULT_STROBE.0,
                to: DEFAULT_STROBE.1,
                random: false,
            },
            label: "Strobe".into(),
        },
    ]
}

pub(crate) fn finalize_channel(ch: &mut FixtureChannel) {
    if ch.kind == ChanKind::Shutter && ch.caps.is_empty() {
        ch.caps = default_shutter_caps();
    }
    ch.caps.sort_by_key(|c| c.min);
}

/// "#ff8800", "ff8800" oder ein Farbname
pub fn parse_color(s: &str) -> Option<Rgb> {
    let l = s.trim().to_lowercase();
    let named: Option<u32> = match l.as_str() {
        "rot" | "red" => Some(0xff0000),
        "grün" | "gruen" | "green" => Some(0x00ff00),
        "blau" | "blue" => Some(0x0000ff),
        "weiß" | "weiss" | "white" => Some(0xffffff),
        "gelb" | "yellow" => Some(0xffff00),
        "cyan" => Some(0x00ffff),
        "magenta" => Some(0xff00ff),
        "orange" => Some(0xff8000),
        "pink" => Some(0xff69b4),
        "lila" | "violett" | "purple" => Some(0x8000ff),
        "amber" => Some(0xffbf00),
        "uv" => Some(0x6a00ff),
        "warmweiß" | "warmweiss" | "warmwhite" => Some(0xffd8a8),
        "kaltweiß" | "kaltweiss" | "coldwhite" => Some(0xe8f0ff),
        _ => None,
    };
    let v = match named {
        Some(v) => v,
        None => {
            let hex = l.trim_start_matches('#');
            if hex.len() != 6 {
                return None;
            }
            u32::from_str_radix(hex, 16).ok()?
        }
    };
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn parse_range(s: &str) -> Option<(u8, u8)> {
    let (a, b) = s.split_once('-')?;
    let a: u8 = a.trim().parse().ok()?;
    let b: u8 = b.trim().parse().ok()?;
    (a <= b).then_some((a, b))
}

/// "offen", "zu", "strobe 1-25", "zufall 5-20", "farbe #ff0000 [#00ff00]", sonst Text
fn parse_effect(s: &str) -> Result<Effect, String> {
    let l = s.trim().to_lowercase();
    let mut it = l.split_whitespace();
    let word = it.next().unwrap_or("");
    let rest: Vec<&str> = it.collect();
    Ok(match word {
        "" | "aus" | "nichts" | "-" | "none" | "keine" => Effect::Nothing,
        "offen" | "open" | "an" | "on" => Effect::Open,
        "zu" | "geschlossen" | "closed" | "blackout" => Effect::Closed,
        "strobe" | "strobo" | "puls" | "pulse" | "zufall" | "random" => {
            let random = matches!(word, "zufall" | "random");
            let raw = rest.join(" ");
            let per_minute = raw.contains("bpm");
            let joined = raw.replace("bpm", " ").replace("hz", " ").replace(',', ".");
            let nums: Result<Vec<f32>, _> = joined
                .split(|c: char| c.is_whitespace() || c == '-' || c == '–')
                .filter(|s| !s.is_empty())
                .map(str::parse::<f32>)
                .collect();
            let nums = nums.map_err(|_| tf!("'{s}': Hz-Angabe unverständlich", "'{s}': frequency not understood"))?;
            let k = if per_minute { 1.0 / 60.0 } else { 1.0 };
            let (from, to) = match nums.as_slice() {
                [] => DEFAULT_STROBE,
                [a] => (*a * k, *a * k),
                [a, b, ..] => (*a * k, *b * k),
            };
            Effect::Strobe { from, to, random }
        }
        "farbe" | "color" | "colour" => {
            let cols: Vec<Rgb> = rest.iter().filter_map(|c| parse_color(c)).collect();
            match cols.as_slice() {
                // "Colour wheel rotation" o. Ä. ist freier Text, kein Farbfehler
                [] if !rest.iter().any(|r| r.starts_with('#')) => Effect::Text,
                [] => return Err(tf!("'{s}': Farbe als #rrggbb oder Name angeben", "'{s}': give a colour as #rrggbb or a name")),
                [a] => Effect::Color(*a, None),
                [a, b, ..] => Effect::Color(*a, Some(*b)),
            }
        }
        _ => Effect::Text,
    })
}

// ---------------------------------------------------------------------------
// Fixtures und Patch
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct FixtureMode {
    pub name: String,
    pub channels: Vec<FixtureChannel>,
}

#[derive(Clone, Debug)]
pub struct FixtureType {
    pub id: String,
    pub name: String,
    pub modes: Vec<FixtureMode>,
}

#[derive(Clone, Debug)]
pub struct PatchedFixture {
    pub name: String,
    pub type_id: String,
    pub type_name: String,
    pub mode_name: String,
    pub universe: u16,
    /// 1-basiert, wie auf dem Gerät eingestellt
    pub address: u16,
    pub channels: Vec<FixtureChannel>,
    pub has_dimmer: bool,
}

impl PatchedFixture {
    /// 0-basierter Slot des Kanals `c` im Universum
    pub fn slot(&self, c: usize) -> usize {
        self.address as usize - 1 + c
    }

    /// Master/Blackout: Hat das Gerät einen Dimmer, wird nur der gedimmt
    /// (Farben bleiben sauber), sonst die additiven Farbkanäle.
    pub fn master_affects(&self, c: usize) -> bool {
        let k = self.channels[c].kind;
        if self.has_dimmer {
            k == ChanKind::Dimmer
        } else {
            k.is_additive()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiTarget {
    pub universe: u16,
    /// 0..=3
    pub block: u8,
}

impl MidiTarget {
    pub fn first_slot(&self) -> usize {
        self.block as usize * BLOCK_SIZE
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub level: Level,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Show {
    pub dir: PathBuf,
    pub artnet_ip: Ipv4Addr,
    pub artnet_port: u16,
    pub refresh_hz: f32,
    pub cc_input: bool,
    pub midi: [Option<MidiTarget>; 16],
    pub fixtures: Vec<PatchedFixture>,
    pub messages: Vec<Message>,
    /// Patch-Zeilen wie in patch.txt (für den Editor)
    pub patch: Vec<PatchEntry>,
    /// alle Fixture-Typen aus fixtures/
    pub types: Vec<TypeInfo>,
    /// Universum → 512 Slots → (Fixture-Index, Kanal-Index)
    slots: HashMap<u16, Vec<Option<(usize, usize)>>>,
}

/// Eine Zeile aus patch.txt
#[derive(Clone, Debug, PartialEq)]
pub struct PatchEntry {
    pub name: String,
    pub fixture: String,
    pub universe: u16,
    pub address: u16,
    pub mode: Option<String>,
    pub count: usize,
    pub spacing: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct TypeInfo {
    pub ft: FixtureType,
    pub file: String,
    pub is_qxf: bool,
}

/// Bearbeitbare Grundeinstellungen (config.txt)
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub artnet_ip: Ipv4Addr,
    pub artnet_port: u16,
    pub refresh_hz: f32,
    pub cc_input: bool,
    pub midi: [Option<MidiTarget>; 16],
}

impl Show {
    pub fn settings(&self) -> Settings {
        Settings {
            artnet_ip: self.artnet_ip,
            artnet_port: self.artnet_port,
            refresh_hz: self.refresh_hz,
            cc_input: self.cc_input,
            midi: self.midi,
        }
    }

    pub fn type_info(&self, id: &str) -> Option<&TypeInfo> {
        self.types.iter().find(|t| t.ft.id == id)
    }

    /// Erste freie Adresse für `len` Kanäle in einem Universum
    /// (berücksichtigt die übergebenen Patch-Zeilen, nicht nur den geladenen Stand).
    pub fn next_free_address(&self, entries: &[PatchEntry], universe: u16, len: usize) -> Option<u16> {
        let mut used = vec![false; DMX_SIZE];
        for e in entries.iter().filter(|e| e.universe == universe) {
            let n = self.entry_channels(e).max(1);
            let step = e.spacing.unwrap_or(n);
            for i in 0..e.count.max(1) {
                let start = e.address as usize - 1 + i * step;
                for s in start..(start + n).min(DMX_SIZE) {
                    used[s] = true;
                }
            }
        }
        let len = len.max(1);
        (0..=DMX_SIZE.saturating_sub(len))
            .find(|&a| used[a..a + len].iter().all(|u| !u))
            .map(|a| a as u16 + 1)
    }

    /// Kanalanzahl einer Patch-Zeile (0, wenn Fixture/Modus unbekannt)
    pub fn entry_channels(&self, e: &PatchEntry) -> usize {
        self.type_info(&e.fixture)
            .and_then(|t| pick_mode(&t.ft, e.mode.as_deref()))
            .map_or(0, |m| m.channels.len())
    }
}

pub fn pick_mode<'a>(ft: &'a FixtureType, req: Option<&str>) -> Option<&'a FixtureMode> {
    match req {
        Some(req) => {
            let r = req.to_lowercase();
            ft.modes
                .iter()
                .find(|m| m.name.to_lowercase() == r)
                .or_else(|| ft.modes.iter().find(|m| m.name.to_lowercase().contains(&r)))
        }
        None => ft.modes.first(),
    }
}

impl Show {
    fn empty(dir: PathBuf) -> Self {
        Self {
            dir,
            artnet_ip: Ipv4Addr::BROADCAST,
            artnet_port: 6454,
            refresh_hz: 40.0,
            cc_input: true,
            midi: default_midi_map(),
            fixtures: Vec::new(),
            messages: Vec::new(),
            patch: Vec::new(),
            types: Vec::new(),
            slots: HashMap::new(),
        }
    }

    fn msg(&mut self, level: Level, text: impl Into<String>) {
        self.messages.push(Message {
            level,
            text: text.into(),
        });
    }

    /// (Ruhewert, Master wirkt) für einen 0-basierten Slot
    pub fn slot_info(&self, universe: u16, slot: usize) -> (u8, bool) {
        match self.lookup_idx(universe, slot) {
            Some((f, c)) => {
                let fx = &self.fixtures[f];
                (fx.channels[c].rest, fx.master_affects(c))
            }
            None => (0, false),
        }
    }

    fn lookup_idx(&self, universe: u16, slot: usize) -> Option<(usize, usize)> {
        self.slots.get(&universe)?.get(slot).copied().flatten()
    }

    pub fn lookup(&self, universe: u16, slot: usize) -> Option<(&PatchedFixture, &FixtureChannel)> {
        let (f, c) = self.lookup_idx(universe, slot)?;
        let fx = self.fixtures.get(f)?;
        Some((fx, fx.channels.get(c)?))
    }

    /// Welche MIDI-Kanal/Note-Kombination steuert diesen Slot? (Kanal 0-basiert)
    pub fn midi_for(&self, universe: u16, slot: usize) -> Option<(u8, u8)> {
        self.midi.iter().enumerate().find_map(|(ch, t)| match t {
            Some(t) if t.universe == universe && t.block as usize == slot / BLOCK_SIZE => {
                Some((ch as u8, (slot % BLOCK_SIZE) as u8))
            }
            _ => None,
        })
    }

    pub fn error_count(&self) -> usize {
        self.messages.iter().filter(|m| m.level == Level::Error).count()
    }
}

pub fn default_midi_map() -> [Option<MidiTarget>; 16] {
    let mut m = [None; 16];
    for (ch, slot) in m.iter_mut().enumerate().take(8) {
        *slot = Some(MidiTarget {
            universe: (ch / 4) as u16,
            block: (ch % 4) as u8,
        });
    }
    m
}

/// MIDI-Notenname, Reaper-Standard (Note 60 = C4)
pub fn note_name(n: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[n as usize % 12], n as i32 / 12 - 1)
}

pub fn default_config_dir() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join("Documents").join("DMX-Bridge")
}

// ---------------------------------------------------------------------------
// Laden
// ---------------------------------------------------------------------------

/// Lädt den kompletten Show-Ordner. Schlägt nie fehl – Probleme landen in
/// `show.messages`, im Zweifel gelten Standardwerte.
pub fn load_show(dir: &Path) -> Show {
    crate::i18n::set_english(read_ui_prefs(dir).english);
    let mut show = Show::empty(dir.to_path_buf());

    if !dir.exists() {
        match write_examples(dir) {
            Ok(()) => show.msg(Level::Info, t("Konfig-Ordner war leer – Beispieldateien angelegt.", "Config folder was empty – example files created.")),
            Err(e) => {
                show.msg(
                    Level::Error,
                    tf!("Konnte Ordner {} nicht anlegen: {e}", "Could not create folder {}: {e}", dir.display()),
                );
                return show;
            }
        }
    }

    match read_text(&dir.join("config.txt")) {
        Ok(t) => parse_config(&mut show, &t),
        Err(e) => show.msg(Level::Warn, tf!("config.txt: {e} – Standardwerte aktiv", "config.txt: {e} – using defaults")),
    }

    let types = load_fixture_types(&mut show, &dir.join("fixtures"));

    match read_text(&dir.join("patch.txt")) {
        Ok(t) => parse_patch(&mut show, &t, &types),
        Err(e) => show.msg(Level::Warn, tf!("patch.txt: {e} – nichts gepatcht", "patch.txt: {e} – nothing patched")),
    }
    let mut infos: Vec<TypeInfo> = types.into_values().collect();
    infos.sort_by(|a, b| a.ft.name.to_lowercase().cmp(&b.ft.name.to_lowercase()));
    let n_types = infos.len();
    show.types = infos;

    validate(&mut show);
    let info = tf!(
        "{n_types} Fixture-Typen, {} Geräte gepatcht",
        "{n_types} fixture types, {} devices patched",
        show.fixtures.len()
    );
    show.msg(Level::Info, info);
    show
}

/// Billiger Fingerabdruck (Größe + Änderungszeit) aller relevanten Dateien,
/// damit gespeicherte Änderungen automatisch neu geladen werden.
pub fn fingerprint(dir: &Path) -> u64 {
    fn meta(p: &Path, h: &mut DefaultHasher) {
        match fs::metadata(p) {
            Ok(m) => {
                m.len().hash(h);
                if let Ok(t) = m.modified() {
                    t.hash(h);
                }
            }
            Err(_) => 0u8.hash(h),
        }
    }
    let mut h = DefaultHasher::new();
    dir.hash(&mut h);
    meta(&dir.join("config.txt"), &mut h);
    meta(&dir.join("patch.txt"), &mut h);
    if let Ok(rd) = fs::read_dir(dir.join("fixtures")) {
        let mut paths: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for p in paths {
            p.hash(&mut h);
            meta(&p, &mut h);
        }
    }
    h.finish()
}

fn write_examples(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir.join("fixtures"))?;
    for (rel, content) in EXAMPLES {
        let p = dir.join(rel);
        if !p.exists() {
            fs::write(p, content)?;
        }
    }
    Ok(())
}

/// UTF-8 (mit/ohne BOM), Fallback Latin-1 für alte Windows-Editoren.
fn read_text(p: &Path) -> Result<String, String> {
    let bytes = fs::read(p).map_err(|e| e.to_string())?;
    let s = match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => e.into_bytes().iter().map(|&b| b as char).collect(),
    };
    Ok(s.trim_start_matches('\u{feff}').to_string())
}

/// (Zeilennummer, Inhalt ohne Kommentar), leere Zeilen übersprungen
fn clean_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines()
        .enumerate()
        .map(|(i, l)| (i + 1, strip_comment(l).trim()))
        .filter(|(_, l)| !l.is_empty())
}

/// '#' leitet einen Kommentar ein – außer direkt vor einer Hex-Farbe (#ff0000)
fn strip_comment(l: &str) -> &str {
    let b = l.as_bytes();
    let mut i = 0;
    while let Some(p) = l[i..].find('#') {
        let pos = i + p;
        let hex = b.len() >= pos + 7
            && b[pos + 1..pos + 7].iter().all(|c| c.is_ascii_hexdigit())
            && b.get(pos + 7).map_or(true, |c| !c.is_ascii_alphanumeric());
        if !hex {
            return &l[..pos];
        }
        i = pos + 1;
    }
    l
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_lowercase().as_str() {
        "an" | "on" | "ja" | "yes" | "true" | "1" => Some(true),
        "aus" | "off" | "nein" | "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

fn parse_target(v: &str) -> Result<Option<MidiTarget>, String> {
    let l = v.trim().to_lowercase();
    if matches!(l.as_str(), "aus" | "off" | "-" | "none") {
        return Ok(None);
    }
    let parts: Vec<&str> = l
        .split(|c: char| c.is_whitespace() || matches!(c, '/' | ',' | ':' | ';'))
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() != 2 {
        return Err(tf!("'{v}': erwartet '<universum> <block>' oder 'aus'", "'{v}': expected '<universe> <block>' or 'off'"));
    }
    let universe: u16 = parts[0]
        .trim_start_matches('u')
        .parse()
        .ok()
        .filter(|u| *u <= MAX_UNIVERSE)
        .ok_or_else(|| tf!("'{}': Universum muss 0–{MAX_UNIVERSE} sein", "'{}': universe must be 0–{MAX_UNIVERSE}", parts[0]))?;
    let block: u8 = parts[1]
        .trim_start_matches('b')
        .parse()
        .ok()
        .filter(|b| (1..=4).contains(b))
        .ok_or_else(|| tf!("'{}': Block muss 1–4 sein", "'{}': block must be 1–4", parts[1]))?;
    Ok(Some(MidiTarget {
        universe,
        block: block - 1,
    }))
}

fn parse_config(show: &mut Show, text: &str) {
    let mut midi = [None; 16];
    let mut midi_seen = false;

    for (ln, line) in clean_lines(text) {
        let Some((k, v)) = line.split_once('=') else {
            show.msg(Level::Warn, tf!("config.txt Zeile {ln}: erwartet 'schlüssel = wert'", "config.txt line {ln}: expected 'key = value'"));
            continue;
        };
        let key = k.trim().to_lowercase();
        let v = v.trim();
        match key.as_str() {
            "format" => {}
            "artnet_ip" | "ip" => match v.parse::<Ipv4Addr>() {
                Ok(ip) => show.artnet_ip = ip,
                Err(_) => show.msg(Level::Error, tf!("config.txt Zeile {ln}: ungültige IP '{v}'", "config.txt line {ln}: invalid IP '{v}'")),
            },
            "artnet_port" | "port" => match v.parse::<u16>() {
                Ok(p) if p > 0 => show.artnet_port = p,
                _ => show.msg(Level::Error, tf!("config.txt Zeile {ln}: ungültiger Port '{v}'", "config.txt line {ln}: invalid port '{v}'")),
            },
            "refresh_hz" => match v.replace(',', ".").parse::<f32>() {
                Ok(hz) if hz.is_finite() => show.refresh_hz = hz.clamp(1.0, 44.0),
                _ => show.msg(Level::Error, tf!("config.txt Zeile {ln}: ungültige Rate '{v}'", "config.txt line {ln}: invalid rate '{v}'")),
            },
            "cc_eingang" | "cc_input" => match parse_bool(v) {
                Some(b) => show.cc_input = b,
                None => show.msg(Level::Error, tf!("config.txt Zeile {ln}: 'an' oder 'aus' erwartet", "config.txt line {ln}: expected 'on' or 'off'")),
            },
            _ if key.starts_with("midi") => {
                let num = key[4..].trim().trim_start_matches(['_', ' ']).parse::<usize>();
                match num {
                    Ok(n) if (1..=16).contains(&n) => match parse_target(v) {
                        Ok(t) => {
                            midi[n - 1] = t;
                            midi_seen = true;
                        }
                        Err(e) => show.msg(Level::Error, tf!("config.txt Zeile {ln}: {e}", "config.txt line {ln}: {e}")),
                    },
                    _ => show.msg(Level::Error, tf!("config.txt Zeile {ln}: MIDI-Kanal muss 1–16 sein", "config.txt line {ln}: MIDI channel must be 1–16")),
                }
            }
            _ => show.msg(
                Level::Warn,
                tf!("config.txt Zeile {ln}: unbekannter Schlüssel '{}' (ignoriert)", "config.txt line {ln}: unknown key '{}' (ignored)", k.trim()),
            ),
        }
    }

    // Sobald irgendein midiN-Eintrag existiert, gilt nur die Datei.
    if midi_seen {
        show.midi = midi;
    }
}

fn load_fixture_types(show: &mut Show, dir: &Path) -> HashMap<String, TypeInfo> {
    let mut map = HashMap::new();
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            show.msg(Level::Warn, tf!("Ordner fixtures/ nicht lesbar: {e}", "Folder fixtures/ not readable: {e}"));
            return map;
        }
    };
    let mut paths: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();

    for p in paths {
        let ext = p
            .extension()
            .map(|x| x.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if ext != "txt" && ext != "qxf" {
            continue;
        }
        let id = p
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let fname = p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if map.contains_key(&id) {
            show.msg(Level::Warn, tf!("fixtures/{fname}: Name '{id}' doppelt, ignoriert", "fixtures/{fname}: name '{id}' used twice, ignored"));
            continue;
        }
        let text = match read_text(&p) {
            Ok(t) => t,
            Err(e) => {
                show.msg(Level::Error, format!("fixtures/{fname}: {e}"));
                continue;
            }
        };
        if ext == "qxf" {
            match qxf::parse_qxf(&id, &text) {
                Ok((ft, warnings)) => {
                    for w in warnings {
                        show.msg(Level::Warn, format!("fixtures/{fname}: {w}"));
                    }
                    map.insert(id, TypeInfo { ft, file: fname, is_qxf: true });
                }
                Err(e) => show.msg(Level::Error, format!("fixtures/{fname}: {e}")),
            }
        } else if let Some(ft) = parse_fixture(show, &id, &fname, &text) {
            map.insert(id, TypeInfo { ft, file: fname, is_qxf: false });
        }
    }
    map
}

fn parse_fixture(show: &mut Show, id: &str, fname: &str, text: &str) -> Option<FixtureType> {
    let mut name = id.to_string();
    let mut chans: Vec<Option<FixtureChannel>> = Vec::new();
    let mut current: Option<usize> = None;

    for (ln, line) in clean_lines(text) {
        let here = |m: &str| tf!("fixtures/{fname} Zeile {ln}: {m}", "fixtures/{fname} line {ln}: {m}");

        if !line.contains('|') {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim().eq_ignore_ascii_case("name") {
                    name = v.trim().to_string();
                }
                // andere Schlüssel: für spätere Versionen reserviert
                continue;
            }
            show.msg(Level::Warn, here(t("unverständlich, ignoriert", "not understood, ignored")));
            continue;
        }

        let parts: Vec<&str> = line.split('|').map(str::trim).collect();

        // --- Wertebereich für den vorigen Kanal: "10-255 | strobe 1-25 | Text"
        if parts[0].contains('-') {
            let Some((min, max)) = parse_range(parts[0]) else {
                show.msg(Level::Error, here(&tf!("Bereich '{}' ungültig (z. B. 10-255)", "range '{}' invalid (e.g. 10-255)", parts[0])));
                continue;
            };
            let Some(ci) = current else {
                show.msg(Level::Error, here(t("Bereich ohne vorherige Kanalzeile", "range without a channel line above")));
                continue;
            };
            let eff_str = parts.get(1).copied().unwrap_or("");
            let effect = match parse_effect(eff_str) {
                Ok(e) => e,
                Err(e) => {
                    show.msg(Level::Error, here(&e));
                    continue;
                }
            };
            let label = parts
                .get(2)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| eff_str.to_string());
            if let Some(Some(ch)) = chans.get_mut(ci) {
                ch.caps.push(Capability { min, max, effect, label });
            }
            continue;
        }

        // --- Kanalzeile: "1 | dimmer | Dimmer | 255"
        let n = match parts[0].parse::<usize>() {
            Ok(n) if (1..=DMX_SIZE).contains(&n) => n,
            _ => {
                show.msg(Level::Error, here(&tf!("Kanalnummer '{}' ungültig", "channel number '{}' invalid", parts[0])));
                continue;
            }
        };
        let kind_str = parts.get(1).copied().unwrap_or("");
        let Some(kind) = ChanKind::parse(kind_str) else {
            show.msg(
                Level::Error,
                here(&tf!(
                    "unbekannter Typ '{kind_str}' (erlaubt: {})",
                    "unknown type '{kind_str}' (allowed: {})",
                    ChanKind::keyword_list()
                )),
            );
            current = None;
            continue;
        };
        let label = parts
            .get(2)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| kind.label().to_string());
        let rest = match parts.get(3).filter(|s| !s.is_empty()) {
            None => 0,
            Some(s) => s.parse::<u8>().unwrap_or_else(|_| {
                show.msg(Level::Error, here(&tf!("Ruhewert '{s}' muss 0–255 sein", "rest value '{s}' must be 0–255")));
                0
            }),
        };

        if chans.len() < n {
            chans.resize(n, None);
        }
        if chans[n - 1].is_some() {
            show.msg(Level::Warn, here(&tf!("Kanal {n} doppelt, letzter gilt", "channel {n} defined twice, last one wins")));
        }
        chans[n - 1] = Some(FixtureChannel {
            kind,
            label,
            rest,
            caps: Vec::new(),
        });
        current = Some(n - 1);
    }

    if chans.is_empty() {
        show.msg(Level::Error, tf!("fixtures/{fname}: keine Kanäle definiert", "fixtures/{fname}: no channels defined"));
        return None;
    }

    let channels = chans
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let mut ch = c.unwrap_or_else(|| FixtureChannel {
                kind: ChanKind::Other,
                label: tf!("Kanal {}", "Channel {}", i + 1),
                rest: 0,
                caps: Vec::new(),
            });
            finalize_channel(&mut ch);
            ch
        })
        .collect();

    Some(FixtureType {
        id: id.to_string(),
        name,
        modes: vec![FixtureMode {
            name: "Standard".into(),
            channels,
        }],
    })
}

fn parse_patch(show: &mut Show, text: &str, types: &HashMap<String, TypeInfo>) {
    for (ln, line) in clean_lines(text) {
        let parts: Vec<&str> = line.split('|').map(str::trim).collect();
        if parts.len() < 4 {
            show.msg(
                Level::Error,
                tf!("patch.txt Zeile {ln}: erwartet 'Name | Fixture | Universum | Adresse'", "patch.txt line {ln}: expected 'name | fixture | universe | address'"),
            );
            continue;
        }
        let tid_lower = parts[1].to_lowercase();
        let tid = tid_lower
            .trim_end_matches(".txt")
            .trim_end_matches(".qxf")
            .to_string();
        let universe = match parts[2].parse::<u16>() {
            Ok(u) if u <= MAX_UNIVERSE => u,
            _ => {
                show.msg(Level::Error, tf!("patch.txt Zeile {ln}: Universum '{}' ungültig", "patch.txt line {ln}: universe '{}' invalid", parts[2]));
                continue;
            }
        };
        let address = match parts[3].parse::<usize>() {
            Ok(a) if (1..=DMX_SIZE).contains(&a) => a,
            _ => {
                show.msg(Level::Error, tf!("patch.txt Zeile {ln}: Adresse '{}' muss 1–512 sein", "patch.txt line {ln}: address '{}' must be 1–512", parts[3]));
                continue;
            }
        };

        // optionale Spalten: Modus, anzahl=n / xN, abstand=n
        let mut mode_req: Option<String> = None;
        let mut count = 1usize;
        let mut spacing: Option<usize> = None;
        for extra in parts.iter().skip(4).filter(|s| !s.is_empty()) {
            if let Some((k, v)) = extra.split_once('=') {
                let v = v.trim();
                match k.trim().to_lowercase().as_str() {
                    "modus" | "mode" => mode_req = Some(v.to_string()),
                    "anzahl" | "count" => match v.parse::<usize>() {
                        Ok(n) if (1..=DMX_SIZE).contains(&n) => count = n,
                        _ => show.msg(Level::Error, tf!("patch.txt Zeile {ln}: anzahl '{v}' ungültig", "patch.txt line {ln}: count '{v}' invalid")),
                    },
                    "abstand" | "spacing" => match v.parse::<usize>() {
                        Ok(n) if (1..=DMX_SIZE).contains(&n) => spacing = Some(n),
                        _ => show.msg(Level::Error, tf!("patch.txt Zeile {ln}: abstand '{v}' ungültig", "patch.txt line {ln}: spacing '{v}' invalid")),
                    },
                    other => show.msg(Level::Warn, tf!("patch.txt Zeile {ln}: '{other}' unbekannt (ignoriert)", "patch.txt line {ln}: '{other}' unknown (ignored)")),
                }
            } else if let Some(n) = extra
                .strip_prefix(['x', 'X', '×'])
                .and_then(|n| n.trim().parse::<usize>().ok())
            {
                count = n.clamp(1, DMX_SIZE);
            } else {
                mode_req = Some(extra.to_string());
            }
        }

        show.patch.push(PatchEntry {
            name: parts[0].to_string(),
            fixture: tid.clone(),
            universe,
            address: address as u16,
            mode: mode_req.clone(),
            count,
            spacing,
        });

        let Some(ft) = types.get(&tid).map(|t| &t.ft) else {
            show.msg(
                Level::Error,
                tf!("patch.txt Zeile {ln}: Fixture '{tid}' nicht in fixtures/ gefunden", "patch.txt line {ln}: fixture '{tid}' not found in fixtures/"),
            );
            continue;
        };

        let mode_list = || {
            ft.modes
                .iter()
                .map(|m| format!("'{}'", m.name))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mode = pick_mode(ft, mode_req.as_deref());
        let Some(mode) = mode else {
            show.msg(
                Level::Error,
                tf!(
                    "patch.txt Zeile {ln}: Modus '{}' gibt es bei '{tid}' nicht (verfügbar: {})",
                    "patch.txt line {ln}: '{tid}' has no mode '{}' (available: {})",
                    mode_req.unwrap_or_default(),
                    mode_list()
                ),
            );
            continue;
        };
        if mode_req.is_none() && ft.modes.len() > 1 {
            show.msg(
                Level::Info,
                tf!(
                    "patch.txt Zeile {ln}: '{tid}' hat mehrere Modi, nehme '{}' (verfügbar: {})",
                    "patch.txt line {ln}: '{tid}' has several modes, using '{}' (available: {})",
                    mode.name,
                    mode_list()
                ),
            );
        }
        if mode.channels.is_empty() {
            show.msg(Level::Error, tf!("patch.txt Zeile {ln}: Modus '{}' hat keine Kanäle", "patch.txt line {ln}: mode '{}' has no channels", mode.name));
            continue;
        }

        let len = mode.channels.len();
        let step = spacing.unwrap_or(len);
        let has_dimmer = mode.channels.iter().any(|c| c.kind == ChanKind::Dimmer);
        for i in 0..count {
            let addr = address + i * step;
            let last = addr + len - 1;
            let name = if count > 1 {
                format!("{} {}", parts[0], i + 1)
            } else {
                parts[0].to_string()
            };
            if last > DMX_SIZE {
                show.msg(
                    Level::Error,
                    tf!("patch.txt Zeile {ln}: '{name}' ({len} Kanäle ab {addr}) endet bei {last} > 512", "patch.txt line {ln}: '{name}' ({len} channels from {addr}) ends at {last} > 512"),
                );
                break;
            }
            show.fixtures.push(PatchedFixture {
                name,
                type_id: tid.clone(),
                type_name: ft.name.clone(),
                mode_name: mode.name.clone(),
                universe,
                address: addr as u16,
                channels: mode.channels.clone(),
                has_dimmer,
            });
        }
    }
}

fn validate(show: &mut Show) {
    let mut slots: HashMap<u16, Vec<Option<(usize, usize)>>> = HashMap::new();
    let mut msgs: Vec<Message> = Vec::new();
    let mut overlap_pairs: HashSet<(usize, usize)> = HashSet::new();

    for (fi, f) in show.fixtures.iter().enumerate() {
        let row = slots.entry(f.universe).or_insert_with(|| vec![None; DMX_SIZE]);
        for ci in 0..f.channels.len() {
            let s = f.slot(ci);
            match row[s] {
                Some((other, _)) => {
                    if overlap_pairs.insert((other, fi)) {
                        msgs.push(Message {
                            level: Level::Warn,
                            text: tf!(
                                "Überschneidung: '{}' und '{}' ab U{} DMX {} (erstes Gerät gewinnt)",
                                "Overlap: '{}' and '{}' from U{} DMX {} (first device wins)",
                                show.fixtures[other].name,
                                f.name,
                                f.universe,
                                s + 1
                            ),
                        });
                    }
                }
                None => row[s] = Some((fi, ci)),
            }
        }

        let unreachable = (0..f.channels.len())
            .filter(|&c| show.midi_for(f.universe, f.slot(c)).is_none())
            .count();
        if unreachable == f.channels.len() {
            msgs.push(Message {
                level: Level::Warn,
                text: format!(
                    "'{}' (U{}) hat keine MIDI-Zuordnung – Universum/Block in config.txt fehlt",
                    f.name, f.universe
                ),
            });
        } else if unreachable > 0 {
            msgs.push(Message {
                level: Level::Warn,
                text: tf!("'{}': {unreachable} Kanäle liegen in einem Block ohne MIDI-Kanal", "'{}': {unreachable} channels are in a block without a MIDI channel", f.name),
            });
        }
    }

    for i in 0..16 {
        for j in (i + 1)..16 {
            if let (Some(a), Some(b)) = (show.midi[i], show.midi[j]) {
                if a == b {
                    msgs.push(Message {
                        level: Level::Warn,
                        text: tf!(
                            "MIDI-Kanal {} und {} steuern denselben Block (U{} Block {})",
                            "MIDI channels {} and {} control the same block (U{} block {})",
                            i + 1,
                            j + 1,
                            a.universe,
                            a.block + 1
                        ),
                    });
                }
            }
        }
    }

    show.slots = slots;
    show.messages.extend(msgs);
}

// ---------------------------------------------------------------------------
// Reaper-Notennamen
// ---------------------------------------------------------------------------

/// Schreibt pro zugeordnetem MIDI-Kanal eine Reaper-Notennamen-Datei,
/// damit im Piano-Roll "PAR 1: Rot" statt "C#-1" steht.
pub fn export_note_names(show: &Show) -> Result<PathBuf, String> {
    let out = show.dir.join("reaper-notennamen");
    fs::create_dir_all(&out).map_err(|e| e.to_string())?;
    let mut written = 0;

    for (ch, t) in show.midi.iter().enumerate() {
        let Some(t) = t else { continue };
        let mut s = String::from("# MIDI note/CC name map\n");
        s.push_str(&format!(
            "# DMX Bridge: MIDI-Kanal {} -> Universum {}, Block {} (DMX {}-{})\n",
            ch + 1,
            t.universe,
            t.block + 1,
            t.first_slot() + 1,
            t.first_slot() + BLOCK_SIZE
        ));
        let mut any = false;
        for n in 0..BLOCK_SIZE {
            if let Some((f, c)) = show.lookup(t.universe, t.first_slot() + n) {
                s.push_str(&format!("{n} {}: {}\n", f.name, c.label));
                any = true;
            }
        }
        if !any {
            continue;
        }
        fs::write(out.join(format!("midi-kanal-{:02}.txt", ch + 1)), s).map_err(|e| e.to_string())?;
        written += 1;
    }

    if written == 0 {
        return Err(t("kein MIDI-Kanal mit gepatchten Geräten", "no MIDI channel with patched devices").into());
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Schreiben (Editor im Plugin)
// ---------------------------------------------------------------------------

/// Schreibt erst in eine Temp-Datei und benennt dann um, damit der
/// Nachlade-Thread nie eine halb geschriebene Datei liest.
/// Die vorherige Version bleibt als `<datei>.bak` liegen.
fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if path.exists() {
        let mut bak = path.as_os_str().to_owned();
        bak.push(".bak");
        let _ = fs::copy(path, PathBuf::from(bak));
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, content).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Text für eine Spalte: keine Trenner, keine Kommentarzeichen
fn cell(s: &str) -> String {
    s.replace('|', "/")
        .replace('#', "Nr.")
        .replace(['\n', '\r', '\t'], " ")
        .trim()
        .to_string()
}

pub fn file_slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.to_lowercase().chars() {
        let c = match c {
            'ä' => 'a',
            'ö' => 'o',
            'ü' => 'u',
            'ß' => 's',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            s.push(c);
        } else if !s.is_empty() && !s.ends_with('_') {
            s.push('_');
        }
    }
    let s = s.trim_matches('_').to_string();
    if s.is_empty() {
        "fixture".into()
    } else {
        s
    }
}

pub fn write_patch(dir: &Path, entries: &[PatchEntry]) -> Result<(), String> {
    let mut out = String::from(t(
        "# Patch-Liste – gespeichert vom DMX-Bridge-Editor (Handbearbeitung weiterhin möglich)\n\
         # Name | Fixture | Universum | Startadresse | [modus=…] [anzahl=n] [abstand=n]\n\n",
        "# Patch list – saved by the DMX Bridge editor (editing by hand still works)\n\
         # name | fixture | universe | start address | [mode=…] [count=n] [spacing=n]\n\n",
    ));
    let w_name = entries.iter().map(|e| cell(&e.name).chars().count()).max().unwrap_or(4).max(4);
    let w_fix = entries.iter().map(|e| e.fixture.chars().count()).max().unwrap_or(7).max(7);
    for e in entries {
        let name = cell(&e.name);
        let mut line = format!(
            "{name:<w_name$} | {:<w_fix$} | {} | {}",
            e.fixture, e.universe, e.address
        );
        if let Some(m) = &e.mode {
            line.push_str(&format!(" | {}={}", t("modus", "mode"), cell(m)));
        }
        if e.count > 1 {
            line.push_str(&format!(" | {}={}", t("anzahl", "count"), e.count));
        }
        if let Some(sp) = e.spacing {
            line.push_str(&format!(" | {}={sp}", t("abstand", "spacing")));
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    write_atomic(&dir.join("patch.txt"), &out)
}

pub fn write_config(dir: &Path, s: &Settings) -> Result<(), String> {
    let mut out = String::new();
    out.push_str(t(
        "# DMX Bridge – Grundeinstellungen (gespeichert vom Editor)\nformat = 1\n\n",
        "# DMX Bridge – settings (saved by the editor)\nformat = 1\n\n",
    ));
    out.push_str(t(
        "# Art-Net-Ziel: IP des Nodes (Unicast empfohlen). 255.255.255.255 = Broadcast\n",
        "# Art-Net target: IP of the node (unicast recommended). 255.255.255.255 = broadcast\n",
    ));
    out.push_str(&format!("artnet_ip = {}\nartnet_port = {}\n\n", s.artnet_ip, s.artnet_port));
    out.push_str(t("# Wiederholrate pro Universum in Hz (1–44)\n", "# Refresh rate per universe in Hz (1–44)\n"));
    out.push_str(&format!("refresh_hz = {}\n\n", fmt_num(s.refresh_hz)));
    out.push_str(t(
        "# MIDI-CCs als Werte annehmen (CC-Nummer = Slot wie die Note)\n",
        "# Accept MIDI CCs as values (CC number = slot, like the note)\n",
    ));
    out.push_str(&format!(
        "{} = {}\n\n",
        t("cc_eingang", "cc_input"),
        if s.cc_input { t("an", "on") } else { t("aus", "off") }
    ));
    out.push_str(t(
        "# MIDI-Kanal → Universum + Block (1 = DMX 1–128, 2 = 129–256, 3 = 257–384, 4 = 385–512)\n",
        "# MIDI channel → universe + block (1 = DMX 1–128, 2 = 129–256, 3 = 257–384, 4 = 385–512)\n",
    ));
    for (i, target) in s.midi.iter().enumerate() {
        let key = format!("midi{}", i + 1);
        match target {
            Some(m) => out.push_str(&format!("{key:<6} = {} {}\n", m.universe, m.block + 1)),
            None => out.push_str(&format!("{key:<6} = {}\n", t("aus", "off"))),
        }
    }
    write_atomic(&dir.join("config.txt"), &out)
}

fn fmt_num(x: f32) -> String {
    if (x - x.round()).abs() < 1e-4 {
        format!("{}", x.round() as i64)
    } else {
        format!("{x:.2}").trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

pub fn effect_to_text(e: &Effect) -> String {
    match e {
        Effect::Nothing => t("aus", "off").into(),
        Effect::Open => t("offen", "open").into(),
        Effect::Closed => t("zu", "closed").into(),
        Effect::Strobe { from, to, random } => format!(
            "{} {}-{}",
            if *random { t("zufall", "random") } else { "strobe" },
            fmt_num(*from),
            fmt_num(*to)
        ),
        Effect::Color(a, None) => format!("{} {}", t("farbe", "color"), hex(*a)),
        Effect::Color(a, Some(b)) => format!("{} {} {}", t("farbe", "color"), hex(*a), hex(*b)),
        Effect::Text => "info".into(),
    }
}

pub fn fixture_to_text(name: &str, channels: &[FixtureChannel], source_note: Option<&str>) -> String {
    let mut out = String::new();
    out.push_str(t("# Fixture – gespeichert vom DMX-Bridge-Editor\n", "# Fixture – saved by the DMX Bridge editor\n"));
    if let Some(n) = source_note {
        out.push_str(&format!("# {}\n", cell(n)));
    }
    out.push_str(t(
        "# Kanal | Typ | Bezeichnung | Ruhewert\n#   von-bis | Funktion | Bezeichnung\n\n",
        "# channel | type | label | rest value\n#   from-to | function | label\n\n",
    ));
    out.push_str(&format!("name = {}\n\n", cell(name)));
    for (i, ch) in channels.iter().enumerate() {
        out.push_str(&format!(
            "{:<3} | {:<14} | {}",
            i + 1,
            ch.kind.keyword(),
            cell(&ch.label)
        ));
        if ch.rest != 0 {
            out.push_str(&format!(" | {}", ch.rest));
        }
        out.push('\n');
        for c in &ch.caps {
            out.push_str(&format!(
                "    {}-{} | {} | {}\n",
                c.min,
                c.max,
                effect_to_text(&c.effect),
                cell(&c.label)
            ));
        }
    }
    out
}

/// Speichert ein Fixture im Textformat; gibt die Fixture-ID (Dateiname ohne .txt) zurück.
pub fn write_fixture_txt(
    dir: &Path,
    id: &str,
    name: &str,
    channels: &[FixtureChannel],
    source_note: Option<&str>,
) -> Result<String, String> {
    if channels.is_empty() {
        return Err(t("ein Fixture braucht mindestens einen Kanal", "a fixture needs at least one channel").into());
    }
    let id = file_slug(id);
    write_atomic(
        &dir.join("fixtures").join(format!("{id}.txt")),
        &fixture_to_text(name, channels, source_note),
    )?;
    Ok(id)
}

/// Freie Fixture-ID: `base`, sonst `base_2`, `base_3` …
pub fn unique_fixture_id(dir: &Path, base: &str) -> String {
    let base = file_slug(base);
    let taken = |id: &str| {
        let f = dir.join("fixtures");
        f.join(format!("{id}.txt")).exists() || f.join(format!("{id}.qxf")).exists()
    };
    if !taken(&base) {
        return base;
    }
    (2..)
        .map(|i| format!("{base}_{i}"))
        .find(|id| !taken(id))
        .unwrap_or(base)
}

/// QLC+-Definition aus der Bibliothek in fixtures/ ablegen. Ist dieselbe
/// Datei schon vorhanden, wird sie wiederverwendet. Gibt die ID zurück.
pub fn install_qxf(dir: &Path, base_id: &str, text: &str) -> Result<String, String> {
    let base = file_slug(base_id);
    let fdir = dir.join("fixtures");
    let same = fs::read_to_string(fdir.join(format!("{base}.qxf")))
        .map(|t| t == text)
        .unwrap_or(false);
    if same {
        return Ok(base);
    }
    let id = unique_fixture_id(dir, &base);
    write_atomic(&fdir.join(format!("{id}.qxf")), text)?;
    Ok(id)
}

// ---------------------------------------------------------------------------
// UI preferences (global, all projects): ui.txt (older versions: oberflaeche.txt)
// ---------------------------------------------------------------------------

pub const UI_SCALES: [f32; 6] = [0.5, 0.75, 1.0, 1.25, 1.5, 2.0];

/// How MIDI notes are shown
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NoteFmt {
    #[default]
    Both,
    Number,
    Name,
}

/// Extra info line under each stage tile
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TileInfo {
    #[default]
    Nothing,
    Address,
    Dimmer,
    Midi,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiPrefs {
    pub scale: f32,
    pub bpm: bool,
    pub english: bool,
    pub notes: NoteFmt,
    pub tile: TileInfo,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            scale: 1.0,
            bpm: false,
            english: true,
            notes: NoteFmt::Both,
            tile: TileInfo::Nothing,
        }
    }
}

/// MIDI note as number, name (C4 = 60, like Reaper) or both
pub fn note_text(n: u8, fmt: NoteFmt) -> String {
    match fmt {
        NoteFmt::Number => n.to_string(),
        NoteFmt::Name => note_name(n),
        NoteFmt::Both => format!("{n} ({})", note_name(n)),
    }
}

pub fn read_ui_prefs(dir: &Path) -> UiPrefs {
    let mut p = UiPrefs::default();
    let text = read_text(&dir.join("ui.txt")).or_else(|_| {
        // ältere Versionen: deutsch, Standardsprache war Deutsch
        p.english = false;
        read_text(&dir.join("oberflaeche.txt"))
    });
    let Ok(text) = text else {
        p.english = true;
        return p;
    };
    for (_, line) in clean_lines(&text) {
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim().trim_end_matches('%').trim().to_lowercase();
        match k.trim().to_lowercase().as_str() {
            "skalierung" | "scale" => {
                if let Ok(x) = v.replace(',', ".").parse::<f32>() {
                    let x = if x > 5.0 { x / 100.0 } else { x };
                    p.scale = UI_SCALES
                        .iter()
                        .copied()
                        .min_by(|a, b| (a - x).abs().total_cmp(&(b - x).abs()))
                        .unwrap_or(1.0);
                }
            }
            "frequenz" | "frequency" => p.bpm = v == "bpm",
            "sprache" | "language" => p.english = !matches!(v.as_str(), "de" | "deutsch" | "german"),
            "noten" | "notes" => {
                p.notes = match v.as_str() {
                    "zahl" | "number" => NoteFmt::Number,
                    "name" => NoteFmt::Name,
                    _ => NoteFmt::Both,
                }
            }
            "kachel" | "tile" => {
                p.tile = match v.as_str() {
                    "adresse" | "address" => TileInfo::Address,
                    "dimmer" => TileInfo::Dimmer,
                    "midi" => TileInfo::Midi,
                    _ => TileInfo::Nothing,
                }
            }
            _ => {}
        }
    }
    p
}

pub fn write_ui_prefs(dir: &Path, p: &UiPrefs) -> Result<(), String> {
    let text = format!(
        "# DMX Bridge – user interface (applies to all projects)\n\
         scale = {:.0} %\n\
         frequency = {}\n\
         language = {}\n\
         notes = {}\n\
         tile = {}\n",
        p.scale * 100.0,
        if p.bpm { "bpm" } else { "hz" },
        if p.english { "en" } else { "de" },
        match p.notes {
            NoteFmt::Both => "both",
            NoteFmt::Number => "number",
            NoteFmt::Name => "name",
        },
        match p.tile {
            TileInfo::Nothing => "nothing",
            TileInfo::Address => "address",
            TileInfo::Dimmer => "dimmer",
            TileInfo::Midi => "midi",
        },
    );
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    fs::write(dir.join("ui.txt"), text).map_err(|e| e.to_string())
}
