//! Eingebettete QLC+-Fixture-Bibliothek (Apache-2.0, siehe library/).
//! Wird erst beim ersten Öffnen des Bibliotheks-Browsers entpackt.

use std::borrow::Cow;
use std::sync::OnceLock;

use crate::inflate;

static PACK: &[u8] = include_bytes!("../library/qlcplus-fixtures.pack");
pub const VERSION: &str = include_str!("../library/VERSION.txt");

pub struct LibEntry {
    pub manufacturer: String,
    pub model: String,
    pub kind: String,
    pub file: String,
    start: usize,
    len: usize,
    /// für die Suche: "hersteller modell typ" klein geschrieben
    search: String,
}

pub struct Library {
    data: Vec<u8>,
    pub entries: Vec<LibEntry>,
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
            start,
            len,
            search: format!("{} {} {}", f[0], f[1], f[2]).to_lowercase(),
        });
        pos = start + len;
    }
    Ok(Library { data, entries })
}

impl Library {
    pub fn text(&self, e: &LibEntry) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.data[e.start..e.start + e.len])
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
