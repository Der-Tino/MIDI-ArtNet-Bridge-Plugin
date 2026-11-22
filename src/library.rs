//! Eingebettete QLC+-Fixture-Bibliothek (Apache-2.0, siehe library/).
//! Wird erst beim ersten Öffnen des Bibliotheks-Browsers entpackt.

use std::borrow::Cow;
use std::sync::OnceLock;

use crate::inflate;

static PACK: &[u8] = include_bytes!("../library/qlcplus-fixtures.pack");

/// Own definitions (library/custom), shown next to the QLC+ fixtures
const CUSTOM: &[(&str, &str)] = &[
    (
        "Generic-Wall-Washer-9x15W-RGBW.qxf",
        include_str!("../library/custom/Generic-Wall-Washer-9x15W-RGBW.qxf"),
    ),
    (
        "UKing-Wall-Washer-9x15W-RGBW.qxf",
        include_str!("../library/custom/UKing-Wall-Washer-9x15W-RGBW.qxf"),
    ),
];
pub const VERSION: &str = include_str!("../library/VERSION.txt");

pub struct LibEntry {
    pub manufacturer: String,
    pub model: String,
    pub kind: String,
    pub file: String,
    /// own definition (library/custom) instead of QLC+
    pub custom: bool,
    start: usize,
    len: usize,
    /// für die Suche: "hersteller modell typ" klein geschrieben
    search: String,
}

pub struct Library {
    data: Vec<u8>,
    pub entries: Vec<LibEntry>,
    /// Hersteller (alphabetisch) → Einträge (nach Modell sortiert)
    pub groups: Vec<(String, Vec<usize>)>,
}

static LIB: OnceLock<Result<Library, String>> = OnceLock::new();

pub fn get() -> Result<&'static Library, &'static str> {
    LIB.get_or_init(|| load(PACK)).as_ref().map_err(|e| e.as_str())
}

pub fn is_loaded() -> bool {
    LIB.get().is_some()
}

fn load(pack: &[u8]) -> Result<Library, String> {
    let data = inflate::zlib_decompress(pack)?;
    let mut entries = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let nl = data[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .ok_or("Bibliothek beschädigt (Kopfzeile)")?;
        let head = String::from_utf8_lossy(&data[pos..pos + nl]).into_owned();
        let f: Vec<&str> = head.split('\t').collect();
        if f.len() < 5 {
            return Err("Bibliothek beschädigt (Felder)".into());
        }
        let len: usize = f[4].parse().map_err(|_| "Bibliothek beschädigt (Länge)")?;
        let start = pos + nl + 1;
        if start + len > data.len() {
            return Err("Bibliothek beschädigt (Ende)".into());
        }
        entries.push(LibEntry {
            manufacturer: f[0].to_string(),
            model: f[1].to_string(),
            kind: f[2].to_string(),
            file: f[3].to_string(),
            custom: false,
            start,
            len,
            search: format!("{} {} {}", f[0], f[1], f[2]).to_lowercase(),
        });
        pos = start + len;
    }
    // own definitions: append to the same data buffer
    let mut data = data;
    for (file, text) in CUSTOM {
        let Ok(doc) = crate::qxf::parse_xml(text) else { continue };
        let Some(root) = doc.child("FixtureDefinition") else { continue };
        let get = |n: &str| root.child(n).map(|e| e.text().to_string()).unwrap_or_default();
        let (manufacturer, model, kind) = (get("Manufacturer"), get("Model"), get("Type"));
        let start = data.len();
        data.extend_from_slice(text.as_bytes());
        entries.push(LibEntry {
            search: format!("{manufacturer} {model} {kind}").to_lowercase(),
            manufacturer,
            model,
            kind,
            file: file.to_string(),
            custom: true,
            start,
            len: text.len(),
        });
    }

    let mut map: std::collections::BTreeMap<String, (String, Vec<usize>)> = Default::default();
    for (i, e) in entries.iter().enumerate() {
        let key = e.manufacturer.to_lowercase().replace(['_', '-'], " ");
        map.entry(key)
            .or_insert_with(|| (e.manufacturer.replace('_', " "), Vec::new()))
            .1
            .push(i);
    }
    let groups = map
        .into_values()
        .map(|(name, mut idx)| {
            idx.sort_by_key(|&i| entries[i].model.to_lowercase());
            (name, idx)
        })
        .collect();
    Ok(Library { data, entries, groups })
}

impl Library {
    pub fn text(&self, e: &LibEntry) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.data[e.start..e.start + e.len])
    }

    /// Wie `search`, aber nach Hersteller gruppiert: (Gruppen-Index, Treffer)
    pub fn search_grouped(&self, query: &str) -> Vec<(usize, Vec<usize>)> {
        let words: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        self.groups
            .iter()
            .enumerate()
            .filter_map(|(gi, (_, idx))| {
                let hits: Vec<usize> = idx
                    .iter()
                    .copied()
                    .filter(|&i| words.iter().all(|w| self.entries[i].search.contains(w.as_str())))
                    .collect();
                (!hits.is_empty()).then_some((gi, hits))
            })
            .collect()
    }

    /// Alle Suchwörter müssen vorkommen (Hersteller, Modell oder Typ).
    pub fn search(&self, query: &str, limit: usize) -> Vec<usize> {
        let words: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| words.iter().all(|w| e.search.contains(w.as_str())))
            .map(|(i, _)| i)
            .take(limit)
            .collect()
    }
}

/// Dateiname für die Kopie im fixtures/-Ordner, z. B. "eurolite_led_par_56.qxf"
pub fn slug(manufacturer: &str, model: &str) -> String {
    let raw = format!("{manufacturer} {model}").to_lowercase();
    let mut s = String::new();
    for c in raw.chars() {
        let c = match c {
            'ä' => 'a',
            'ö' => 'o',
            'ü' => 'u',
            'ß' => 's',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            s.push(c);
        } else if !s.ends_with('_') {
            s.push('_');
        }
    }
    s.trim_matches('_').to_string()
}

#[cfg(test)]
pub fn load_for_test(pack: &[u8]) -> Result<Library, String> {
    load(pack)
}
