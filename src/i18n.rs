//! Minimal translation layer: every user-facing text is written in German and
//! English side by side. The language is global (stored in ui.txt).

use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(true);

pub fn set_english(on: bool) {
    ENGLISH.store(on, Ordering::Relaxed);
}

pub fn en() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

/// Pick the text for the current language.
pub fn t(de: &'static str, en: &'static str) -> &'static str {
    if self::en() {
        en
    } else {
        de
    }
}

/// `format!` in two languages: `tf!("Zeile {ln}", "line {ln}")`
#[macro_export]
macro_rules! tf {
    ($de:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if $crate::i18n::en() {
            format!($en $(, $arg)*)
        } else {
            format!($de $(, $arg)*)
        }
    };
}
