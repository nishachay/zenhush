//! Persistent settings & local audio library for Zenhush.
//!
//! Stored as JSON under the user's config directory (`~/.config/zenhush/settings.json`
//! on Linux, `~/Library/Application Support/zenhush/settings.json` on macOS).
//! Includes backward-compatible defaults so upgrading from v1 preserves user settings.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::synth::{
    FileTone, Sound, DEFAULT_DEPTH, DEFAULT_LAYER_MIX, DEFAULT_RUMBLE, DEFAULT_VOLUME,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum ThemeMode {
    #[default]
    Auto,
    Light,
    Dark,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum WindowMode {
    #[default]
    Card,
    NotchPill,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, Default)]
pub enum DockAnchor {
    #[default]
    TopCenter,
    TopRight,
    Free,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalTrack {
    pub path: PathBuf,
    pub title: String,
    #[serde(default)]
    pub duration_secs: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub volume: u8,
    pub muted: bool,
    pub depth: u8,
    pub rumble: u8,

    /// Last active selection (ignored at launch when `locked_default_file` is set).
    pub active_is_file: bool,
    pub active_sound: Sound,
    pub active_file: Option<PathBuf>,

    /// User-locked default local audio file (takes precedence at startup!).
    pub locked_default_file: Option<PathBuf>,
    pub local_tracks: Vec<LocalTrack>,

    pub file_tone: FileTone,
    pub loop_crossfade: bool,

    pub layer_sound: Option<Sound>,
    pub layer_mix: u8,

    pub theme: ThemeMode,
    pub reduce_motion: bool,
    /// Sleep/focus timer in minutes (0 = off).
    pub timer_minutes: u32,

    pub window_mode: WindowMode,
    pub dock_anchor: DockAnchor,
    pub saved_pos: Option<[f32; 2]>,
    pub pinned: bool,
    pub auto_play_on_launch: bool,
    pub start_in_background: bool,
    pub gnome_pill_opted_out: bool,

    pub global_shortcut_toggle: String,
    pub global_shortcut_play: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            volume: DEFAULT_VOLUME,
            muted: false,
            depth: DEFAULT_DEPTH,
            rumble: DEFAULT_RUMBLE,
            active_is_file: false,
            active_sound: Sound::Brown,
            active_file: None,
            locked_default_file: None,
            local_tracks: Vec::new(),
            file_tone: FileTone::Original,
            loop_crossfade: true,
            layer_sound: None,
            layer_mix: DEFAULT_LAYER_MIX,
            theme: ThemeMode::Auto,
            reduce_motion: false,
            timer_minutes: 0,
            window_mode: WindowMode::Card,
            dock_anchor: DockAnchor::TopCenter,
            saved_pos: None,
            pinned: false,
            auto_play_on_launch: false,
            start_in_background: false,
            gnome_pill_opted_out: false,
            global_shortcut_toggle: "Alt+Shift+Z".to_string(),
            global_shortcut_play: "Alt+Shift+Space".to_string(),
        }
    }
}

fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("zenhush").join("settings.json"))
}

pub fn load() -> Settings {
    let Some(path) = settings_path() else {
        return Settings::default();
    };
    let Ok(text) = fs::read_to_string(&path) else {
        return Settings::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save(settings: &Settings) {
    let Some(path) = settings_path() else {
        return;
    };
    save_to_path(&path, settings);
}

fn save_to_path(path: &std::path::Path, settings: &Settings) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let Ok(text) = serde_json::to_string_pretty(settings) else {
        return;
    };
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    if fs::write(&tmp, text).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_settings_deserialize_with_clean_defaults() {
        let v1 = r#"{
            "volume": 55,
            "depth": 80,
            "rumble": 40,
            "theme": "Dark",
            "reduce_motion": true,
            "timer_minutes": 25
        }"#;
        let s: Settings = serde_json::from_str(v1).unwrap();
        assert_eq!(s.volume, 55);
        assert_eq!(s.depth, 80);
        assert_eq!(s.theme, ThemeMode::Dark);
        assert!(s.reduce_motion);
        assert_eq!(s.timer_minutes, 25);
        assert!(s.locked_default_file.is_none());
        assert_eq!(s.file_tone, FileTone::Original);
    }

    #[test]
    fn atomic_save_roundtrips_cleanly() {
        let dir = std::env::temp_dir().join(format!("zenhush-cfg-test-{}", std::process::id()));
        let path = dir.join("settings.json");
        let s = Settings {
            volume: 42,
            gnome_pill_opted_out: true,
            ..Settings::default()
        };
        save_to_path(&path, &s);
        let text = fs::read_to_string(&path).unwrap();
        let loaded: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(loaded.volume, 42);
        assert!(loaded.gnome_pill_opted_out);
        let _ = fs::remove_dir_all(&dir);
    }
}
