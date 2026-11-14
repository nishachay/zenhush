//! Settings persistence: volume, depth, rumble, theme, reduce motion,
//! and sleep timer, remembered across restarts in a JSON file under
//! the user config dir (~/.config/zenhush/settings.json on Linux).
//! Missing or corrupt settings fall back to the documented defaults,
//! so a fresh install behaves exactly like the Python app's defaults.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::synth::{DEFAULT_DEPTH, DEFAULT_RUMBLE, DEFAULT_VOLUME};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ThemeMode {
    Auto,
    Light,
    Dark,
}

impl ThemeMode {
    pub const ALL: [ThemeMode; 3] = [ThemeMode::Auto, ThemeMode::Light, ThemeMode::Dark];

    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::Auto => "Auto",
            ThemeMode::Light => "Light",
            ThemeMode::Dark => "Dark",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub volume: u8,
    pub depth: u8,
    pub rumble: u8,
    pub theme: ThemeMode,
    pub reduce_motion: bool,
    /// Sleep timer in minutes (0 = off).
    pub timer_minutes: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            volume: DEFAULT_VOLUME,
            depth: DEFAULT_DEPTH,
            rumble: DEFAULT_RUMBLE,
            theme: ThemeMode::Auto,
            reduce_motion: false,
            timer_minutes: 0,
        }
    }
}

fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("zenhush").join("settings.json"))
}

/// Load settings, falling back to defaults when missing or unreadable.
pub fn load() -> Settings {
    let Some(path) = settings_path() else {
        return Settings::default();
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return Settings::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// Save settings. Errors are ignored (a missing config dir is not fatal).
pub fn save(settings: &Settings) {
    let Some(path) = settings_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(settings) {
        let _ = fs::write(&path, text);
    }
}
