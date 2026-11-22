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

/// Is the operating system set to German? Used as the default language when the
/// user has not chosen one yet. Anything else (or unknown) → English.
pub fn system_prefers_german() -> bool {
    system_locale()
        .map(|l| l.to_lowercase().starts_with("de"))
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn system_locale() -> Option<String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserDefaultLocaleName(lp_locale_name: *mut u16, cch_locale_name: i32) -> i32;
    }
    let mut buf = [0u16; 85]; // LOCALE_NAME_MAX_LENGTH
    // SAFETY: the buffer is valid for the given length
    let n = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
    if n <= 1 {
        return None;
    }
    Some(String::from_utf16_lossy(&buf[..(n - 1) as usize]))
}

#[cfg(target_os = "macos")]
fn system_locale() -> Option<String> {
    // GUI apps usually have no LANG – ask the user defaults (e.g. "de_DE")
    std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLocale"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(env_locale)
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn system_locale() -> Option<String> {
    env_locale()
}

#[allow(dead_code)]
fn env_locale() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty() && v != "C" && v != "POSIX")
}
