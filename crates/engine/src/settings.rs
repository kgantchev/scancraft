//! The application's settings (§4.6), kept in `config.toml` through `craft-settings`.

use crate::error::{EngineError, fs_error};
use craft_settings::{Location, SettingsStore, TickResult};
use scancraft_core::recipe::{PageOrder, Paper};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Instant;

/// The regions that use US Letter paper (Unicode CLDR), §4.6.
const LETTER_REGIONS: [&str; 14] = ["BZ", "CA", "CL", "CO", "CR", "GT", "MX", "NI", "PA", "PH", "PR", "SV", "US", "VE"];
/// The languages written right to left that §4.6 names.
const RTL_LANGUAGES: [&str; 4] = ["ar", "he", "fa", "ur"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The paper of pages that don't snap to a standard size.
    pub default_paper: Paper,
    pub snap_to_paper: bool,
    pub rotate_upright: bool,
    /// New pages start with Flatten on Auto when on, and on Off when off.
    pub flatten_automatically: bool,
    pub book_page_order: PageOrder,
    pub language_correction: bool,
    pub keep_paper_colour: bool,
    /// Where sessions live; `None` for the system default.
    pub sessions_dir: Option<String>,
    /// Book Spreads as the last session left it; it becomes the next session's default (§4.3).
    pub book_spreads: bool,
}

impl Settings {
    /// The defaults of §4.6 for a system whose environment `env` describes (`LANG`-style variables).
    pub fn for_locale(env: impl Fn(&str) -> Option<String>) -> Settings {
        let locale =
            ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|k| env(k).filter(|v| !v.is_empty() && v != "C" && v != "POSIX")).unwrap_or_default();
        // "en_US.UTF-8" → language "en", region "US".
        let tag = locale.split(['.', '@']).next().unwrap_or_default();
        let (language, region) = tag.split_once(['_', '-']).map_or((tag, ""), |(l, r)| (l, r));
        let letter = LETTER_REGIONS.contains(&region.to_ascii_uppercase().as_str());
        let rtl = RTL_LANGUAGES.contains(&language.to_ascii_lowercase().as_str());
        Settings {
            default_paper: if letter { Paper::UsLetter } else { Paper::IsoA(4) },
            snap_to_paper: true,
            rotate_upright: true,
            flatten_automatically: true,
            book_page_order: if rtl { PageOrder::RightToLeft } else { PageOrder::LeftToRight },
            language_correction: true,
            keep_paper_colour: false,
            sessions_dir: None,
            book_spreads: false,
        }
    }
}

impl Default for Settings {
    fn default() -> Settings {
        Settings::for_locale(|k| std::env::var(k).ok())
    }
}

impl craft_settings::Settings for Settings {
    const APP_ID: &'static str = "scancraft";
    const SCHEMA_VERSION: u32 = 1;
}

/// The default sessions folder: `~/Library/Application Support/ScanCraft/Sessions` on a Mac.
pub fn default_sessions_dir() -> PathBuf {
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|h| if cfg!(target_os = "macos") { h.join("Library/Application Support") } else { h.join(".local/share") });
    base.unwrap_or_else(std::env::temp_dir).join("ScanCraft").join("Sessions")
}

/// The settings file plus its in-memory value.
pub struct SettingsHolder {
    store: SettingsStore<Settings>,
}

impl SettingsHolder {
    pub fn open(location: Location) -> SettingsHolder {
        let (store, _report) = SettingsStore::<Settings>::open(location);
        SettingsHolder { store }
    }

    pub fn get(&self) -> &Settings {
        self.store.get()
    }

    /// Changes the settings; they are written on the next `tick`.
    pub fn update(&mut self, f: impl FnOnce(&mut Settings)) -> bool {
        self.store.update(f)
    }

    pub fn tick(&mut self, now: Instant) -> TickResult {
        self.store.tick(now)
    }

    pub fn flush(&mut self) -> Result<(), EngineError> {
        self.store.flush().map_err(|e| fs_error("the settings could not be saved", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| (*v).to_string())
    }

    #[test]
    fn the_defaults_follow_the_system_region_and_language() {
        let us = Settings::for_locale(env(&[("LANG", "en_US.UTF-8")]));
        assert_eq!(us.default_paper, Paper::UsLetter);
        assert_eq!(us.book_page_order, PageOrder::LeftToRight);
        let de = Settings::for_locale(env(&[("LANG", "de_DE.UTF-8")]));
        assert_eq!(de.default_paper, Paper::IsoA(4));
        let ar = Settings::for_locale(env(&[("LC_ALL", "ar_EG.UTF-8"), ("LANG", "en_US")]));
        assert_eq!((ar.default_paper, ar.book_page_order), (Paper::IsoA(4), PageOrder::RightToLeft));
        let mx = Settings::for_locale(env(&[("LANG", "es-MX")]));
        assert_eq!(mx.default_paper, Paper::UsLetter);
        for none in [Settings::for_locale(env(&[])), Settings::for_locale(env(&[("LANG", "C")])), Settings::for_locale(env(&[("LANG", "")]))] {
            assert_eq!((none.default_paper, none.book_page_order), (Paper::IsoA(4), PageOrder::LeftToRight));
        }
        // The other defaults of §4.6.
        assert!(us.snap_to_paper && us.rotate_upright && us.flatten_automatically && us.language_correction && !us.keep_paper_colour);
        assert!(us.sessions_dir.is_none());
    }

    #[test]
    fn the_settings_are_written_and_read_back() {
        let dir = std::env::temp_dir().join(format!("scancraft-engine-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut holder = SettingsHolder::open(Location::Dir(dir.clone()));
        assert!(holder.update(|s| {
            s.keep_paper_colour = true;
            s.default_paper = Paper::UsLegal;
        }));
        holder.flush().unwrap();
        let again = SettingsHolder::open(Location::Dir(dir.clone()));
        assert!(again.get().keep_paper_colour);
        assert_eq!(again.get().default_paper, Paper::UsLegal);
        // Memory-only settings never touch the disk.
        let mut memory = SettingsHolder::open(Location::Memory);
        memory.update(|s| s.snap_to_paper = false);
        assert!(!memory.get().snap_to_paper);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_default_sessions_folder_is_under_the_user_data_folder() {
        let p = default_sessions_dir();
        assert!(p.ends_with("ScanCraft/Sessions"), "{p:?}");
    }
}
