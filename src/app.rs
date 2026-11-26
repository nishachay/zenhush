//! Zenhush UI & UX: Top-Bar / Notch Pill + Full Focus Card,
//! local track library with Lock-as-Default, atmosphere layering,
//! instant volume & mute, and global + in-app keyboard shortcuts.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Color32, CornerRadius, Key, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::audio::AudioEngine;
use crate::filemode::{self, FileStreamHandle};
use crate::icons::{IconCache, IconKind};
use crate::ipc::{IpcServer, RemoteCmd};
use crate::settings::{self, DockAnchor, LocalTrack, Settings, ThemeMode, WindowMode};
use crate::shortcuts::{self, GlobalAction, ShortcutManager};
use crate::synth::{FileTone, Sound, DEFAULT_DEPTH, DEFAULT_RUMBLE, DEFAULT_VOLUME};
use crate::theme::{Theme, DARK, LIGHT};
use crate::tray::{Tray, TrayEvent, TrayState};

const CARD_W: f32 = 336.0;
const CARD_CLOSED_H: f32 = 344.0;
const CARD_OPEN_H: f32 = 594.0;
const NOTCH_W: f32 = 286.0;
const NOTCH_H: f32 = 42.0;

const BAR_COUNT: usize = 18;
const IDLE_HEIGHTS: [f32; BAR_COUNT] = [
    0.26, 0.42, 0.65, 0.50, 0.36, 0.78, 0.92, 0.60, 0.44, 0.85, 0.72, 0.48, 0.62, 0.38, 0.54,
    0.40, 0.32, 0.24,
];

const TIMER_CHOICES: [(u32, &str); 7] = [
    (0, "Off"),
    (15, "15m"),
    (25, "25m"),
    (30, "30m"),
    (45, "45m"),
    (60, "60m"),
    (90, "90m"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum DrawerTab {
    Files,
    MixTimer,
    BarKeys,
}

fn bold_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("SchibstedGroteskBold".into()))
}

fn semibold_font(size: f32) -> egui::FontId {
    egui::FontId::new(
        size,
        egui::FontFamily::Name("SchibstedGroteskSemiBold".into()),
    )
}

fn format_duration(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

pub struct ZenhushApp {
    audio: AudioEngine,
    tray: Option<Tray>,
    ipc: Option<IpcServer>,
    shortcuts: ShortcutManager,

    // Sound & track selection
    is_file_mode: bool,
    active_sound_idx: usize,
    active_file: Option<PathBuf>,
    locked_default_file: Option<PathBuf>,
    local_tracks: Vec<LocalTrack>,
    file_tone: FileTone,
    loop_crossfade: bool,
    layer_sound: Option<Sound>,
    layer_mix: u8,

    // Playback & timers
    playing: bool,
    has_played: bool,
    elapsed: Duration,
    last_tick: Instant,
    timer_minutes: u32,
    timer_remaining: Option<Duration>,
    timer_end: Option<Instant>,

    // UI state
    theme_mode: ThemeMode,
    reduce_motion: bool,
    window_mode: WindowMode,
    dock_anchor: DockAnchor,
    saved_pos: Option<[f32; 2]>,
    pinned: bool,
    auto_play_on_launch: bool,
    start_in_background: bool,
    autostart_enabled: bool,
    global_shortcut_toggle: String,
    global_shortcut_play: String,

    drawer_open: bool,
    drawer_tab: DrawerTab,
    visible: bool,
    was_focused: bool,
    modal_open: bool,
    suppress_hide_until: Instant,
    anchor_applied: bool,
    last_window_size: [f32; 2],
    last_pos_change: Option<Instant>,

    // Visualizer & status
    rms_history: VecDeque<f32>,
    bar_heights: [f32; BAR_COUNT],
    bar_accent: usize,
    icons: IconCache,
    notice: Option<(String, bool)>, // (message, is_error)
    notice_until: Option<Instant>,
    missing_locked_file: Option<PathBuf>,
}

impl ZenhushApp {
    pub fn new(
        mut audio: AudioEngine,
        tray: Option<Tray>,
        ipc: Option<IpcServer>,
        mut settings: Settings,
        start_visible: bool,
    ) -> Self {
        audio.set_volume(settings.volume);
        audio.set_muted(settings.muted);
        audio.set_depth(settings.depth);
        audio.set_rumble(settings.rumble);
        audio.set_layer(settings.layer_sound);
        audio.set_layer_mix(settings.layer_mix);
        audio.set_file_tone(settings.file_tone);

        // Migrate / ensure locked default file is in local_tracks
        if let Some(ref locked) = settings.locked_default_file {
            if !settings.local_tracks.iter().any(|t| &t.path == locked) {
                let title = locked
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Audio Track".to_string());
                let dur = filemode::probe_file(locked)
                    .ok()
                    .and_then(|p| p.duration_secs);
                settings.local_tracks.insert(
                    0,
                    LocalTrack {
                        path: locked.clone(),
                        title,
                        duration_secs: dur,
                    },
                );
            }
        }

        // Resolve initial source: locked default local audio file ALWAYS wins on launch!
        let mut is_file_mode = false;
        let mut active_file = settings.active_file.clone();
        let mut missing_locked_file = None;
        let mut notice = None;

        if let Some(ref locked) = settings.locked_default_file {
            if locked.is_file() {
                is_file_mode = true;
                active_file = Some(locked.clone());
            } else {
                missing_locked_file = Some(locked.clone());
                notice = Some((
                    "Locked default audio file was moved or deleted".to_string(),
                    true,
                ));
            }
        } else if settings.active_is_file {
            if let Some(ref p) = settings.active_file {
                if p.is_file() {
                    is_file_mode = true;
                }
            }
        }

        let active_sound_idx = Sound::ALL
            .iter()
            .position(|&s| s == settings.active_sound)
            .unwrap_or(0);

        let shortcuts = ShortcutManager::new(
            &settings.global_shortcut_toggle,
            &settings.global_shortcut_play,
        );

        let mut app = ZenhushApp {
            audio,
            tray,
            ipc,
            shortcuts,
            is_file_mode,
            active_sound_idx,
            active_file,
            locked_default_file: settings.locked_default_file,
            local_tracks: settings.local_tracks,
            file_tone: settings.file_tone,
            loop_crossfade: settings.loop_crossfade,
            layer_sound: settings.layer_sound,
            layer_mix: settings.layer_mix,
            playing: false,
            has_played: false,
            elapsed: Duration::ZERO,
            last_tick: Instant::now(),
            timer_minutes: settings.timer_minutes,
            timer_remaining: None,
            timer_end: None,
            theme_mode: settings.theme,
            reduce_motion: settings.reduce_motion,
            window_mode: settings.window_mode,
            dock_anchor: settings.dock_anchor,
            saved_pos: settings.saved_pos,
            pinned: settings.pinned,
            auto_play_on_launch: settings.auto_play_on_launch,
            start_in_background: settings.start_in_background,
            autostart_enabled: shortcuts::is_autostart_enabled(),
            global_shortcut_toggle: settings.global_shortcut_toggle,
            global_shortcut_play: settings.global_shortcut_play,
            drawer_open: false,
            drawer_tab: DrawerTab::Files,
            visible: start_visible,
            was_focused: false,
            modal_open: false,
            suppress_hide_until: Instant::now() + Duration::from_millis(500),
            anchor_applied: false,
            last_window_size: match settings.window_mode {
                WindowMode::NotchPill => [NOTCH_W, NOTCH_H],
                WindowMode::Card => [CARD_W, CARD_CLOSED_H],
            },
            last_pos_change: None,
            rms_history: VecDeque::with_capacity(64),
            bar_heights: IDLE_HEIGHTS,
            bar_accent: 6,
            icons: IconCache::new(),
            notice,
            notice_until: None,
            missing_locked_file,
        };

        if app.timer_minutes > 0 {
            app.arm_timer();
        }
        if app.auto_play_on_launch {
            app.play_selected();
        }
        app.sync_tray();
        app
    }

    fn set_notice(&mut self, msg: impl Into<String>, is_error: bool) {
        self.notice = Some((msg.into(), is_error));
        self.notice_until = Some(Instant::now() + Duration::from_secs(if is_error { 6 } else { 3 }));
    }

    fn current_sound(&self) -> Sound {
        Sound::ALL[self.active_sound_idx]
    }

    fn is_current_locked_default(&self) -> bool {
        self.is_file_mode
            && self.active_file.is_some()
            && self.active_file == self.locked_default_file
    }

    fn active_track_title(&self) -> String {
        let Some(path) = &self.active_file else {
            return "Local File".to_string();
        };
        if let Some(t) = self.local_tracks.iter().find(|t| &t.path == path) {
            return t.title.clone();
        }
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Local File".to_string())
    }

    fn display_title(&self) -> String {
        if self.is_file_mode {
            if self.active_file.is_some() {
                truncate_chars(&self.active_track_title(), 22)
            } else {
                "Local Audio".to_string()
            }
        } else {
            self.current_sound().title().to_string()
        }
    }

    fn display_subtitle(&self) -> String {
        if !self.playing && !self.has_played {
            if self.is_current_locked_default() {
                return "locked default • press play".to_string();
            }
            return "press play to focus".to_string();
        }
        if self.is_file_mode {
            if self.is_current_locked_default() {
                "locked default track".to_string()
            } else {
                "local audio loop".to_string()
            }
        } else if let Some(layer) = self.layer_sound {
            format!(
                "{} + {}",
                self.current_sound().subtitle(),
                layer.short_label().to_lowercase()
            )
        } else {
            self.current_sound().subtitle().to_string()
        }
    }

    fn status_badge(&self) -> String {
        if let Some(_err) = self.audio.audio_error() {
            return "No audio device".to_string();
        }
        if let Some(rem) = self.timer_remaining {
            if self.playing || rem.as_millis() > 0 {
                return format_duration(rem.as_secs());
            }
        }
        if !self.has_played {
            return if self.is_current_locked_default() {
                "LOCKED".to_string()
            } else {
                "READY".to_string()
            };
        }
        if !self.playing {
            return "Paused".to_string();
        }
        if self.is_file_mode {
            if self.layer_sound.is_some() {
                "File + Mix".to_string()
            } else if self.is_current_locked_default() {
                "Locked File".to_string()
            } else {
                "File".to_string()
            }
        } else if self.layer_sound.is_some() {
            "Layered".to_string()
        } else {
            "Live".to_string()
        }
    }

    fn save_settings(&self) {
        let s = Settings {
            volume: self.audio.volume(),
            muted: self.audio.is_muted(),
            depth: self.audio.depth(),
            rumble: self.audio.rumble(),
            active_is_file: self.is_file_mode,
            active_sound: self.current_sound(),
            active_file: self.active_file.clone(),
            locked_default_file: self.locked_default_file.clone(),
            local_tracks: self.local_tracks.clone(),
            file_tone: self.file_tone,
            loop_crossfade: self.loop_crossfade,
            layer_sound: self.layer_sound,
            layer_mix: self.layer_mix,
            theme: self.theme_mode,
            reduce_motion: self.reduce_motion,
            timer_minutes: self.timer_minutes,
            window_mode: self.window_mode,
            dock_anchor: self.dock_anchor,
            saved_pos: self.saved_pos,
            pinned: self.pinned,
            auto_play_on_launch: self.auto_play_on_launch,
            start_in_background: self.start_in_background,
            global_shortcut_toggle: self.global_shortcut_toggle.clone(),
            global_shortcut_play: self.global_shortcut_play.clone(),
        };
        settings::save(&s);
    }

    fn sync_tray(&self) {
        let Some(tray) = &self.tray else {
            return;
        };
        let timer_label = self
            .timer_remaining
            .filter(|r| r.as_secs() > 0)
            .map(|r| format_duration(r.as_secs()))
            .unwrap_or_default();

        let state_word = if self.playing {
            "Playing"
        } else if self.has_played {
            "Paused"
        } else {
            "Ready"
        };
        let vol_word = if self.audio.is_muted() {
            "Muted".to_string()
        } else {
            format!("{}%", self.audio.volume())
        };
        let tooltip = format!(
            "Zenhush — {} • {} ({})",
            self.display_title(),
            state_word,
            vol_word
        );

        tray.sync_state(&TrayState {
            playing: self.playing,
            sound_title: self.display_title(),
            is_file: self.is_file_mode,
            has_file: self.active_file.is_some(),
            file_title: truncate_chars(&self.active_track_title(), 22),
            is_locked_default: self.locked_default_file.is_some()
                && self.locked_default_file == self.active_file,
            volume: self.audio.volume(),
            muted: self.audio.is_muted(),
            timer_minutes: self.timer_minutes,
            timer_label,
            notch_mode: self.window_mode == WindowMode::NotchPill,
            pinned: self.pinned,
            visible: self.visible,
            tooltip,
        });
    }

    fn target_size(&self) -> Vec2 {
        match self.window_mode {
            WindowMode::NotchPill => Vec2::new(NOTCH_W, NOTCH_H),
            WindowMode::Card => {
                let mut h = if self.drawer_open {
                    CARD_OPEN_H
                } else {
                    CARD_CLOSED_H
                };
                if self.notice.is_some() || self.missing_locked_file.is_some() {
                    h += 28.0;
                }
                if self.tray.is_none() {
                    h += 24.0;
                }
                Vec2::new(CARD_W, h)
            }
        }
    }

    fn apply_window_geometry(&mut self, ctx: &egui::Context) {
        let size = self.target_size();
        if (size.x - self.last_window_size[0]).abs() > 0.5
            || (size.y - self.last_window_size[1]).abs() > 0.5
        {
            self.last_window_size = [size.x, size.y];
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            if self.dock_anchor != DockAnchor::Free {
                self.anchor_applied = false;
            }
        }

        if !self.anchor_applied {
            if let Some(mon) = ctx.input(|i| i.viewport().monitor_size) {
                let pos = match self.dock_anchor {
                    DockAnchor::TopCenter => {
                        let y = if self.window_mode == WindowMode::NotchPill {
                            6.0
                        } else {
                            36.0
                        };
                        Some(Pos2::new(((mon.x - size.x) * 0.5).max(8.0), y))
                    }
                    DockAnchor::TopRight => Some(Pos2::new((mon.x - size.x - 16.0).max(8.0), 36.0)),
                    DockAnchor::Free => self.saved_pos.map(|[x, y]| Pos2::new(x, y)),
                };
                if let Some(p) = pos {
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(p));
                }
                self.anchor_applied = true;
            }
        }
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        self.visible = true;
        self.was_focused = false;
        self.suppress_hide_until = Instant::now() + Duration::from_millis(350);
        self.anchor_applied = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        self.sync_tray();
    }

    fn hide_window(&mut self, ctx: &egui::Context) {
        if self.tray.is_none() {
            return;
        }
        self.visible = false;
        self.was_focused = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        self.sync_tray();
    }

    fn toggle_window(&mut self, ctx: &egui::Context) {
        if self.visible {
            self.hide_window(ctx);
        } else {
            self.show_window(ctx);
        }
    }

    fn toggle_notch_mode(&mut self, ctx: &egui::Context) {
        self.window_mode = match self.window_mode {
            WindowMode::Card => WindowMode::NotchPill,
            WindowMode::NotchPill => WindowMode::Card,
        };
        if !self.visible {
            self.show_window(ctx);
        }
        self.anchor_applied = false;
        self.apply_window_geometry(ctx);
        self.save_settings();
        self.sync_tray();
    }

    fn add_or_select_local_file(&mut self, path: PathBuf, play_now: bool, lock_default: bool) {
        match filemode::probe_file(&path) {
            Ok(probe) => {
                let title = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Audio Track".to_string());
                if let Some(existing) = self.local_tracks.iter_mut().find(|t| t.path == path) {
                    existing.title = title.clone();
                    if probe.duration_secs.is_some() {
                        existing.duration_secs = probe.duration_secs;
                    }
                } else {
                    self.local_tracks.insert(
                        0,
                        LocalTrack {
                            path: path.clone(),
                            title,
                            duration_secs: probe.duration_secs,
                        },
                    );
                }
                self.active_file = Some(path.clone());
                self.is_file_mode = true;
                self.missing_locked_file = None;
                if lock_default {
                    self.locked_default_file = Some(path.clone());
                    self.set_notice("Locked as default audio", false);
                }
                self.save_settings();
                if play_now {
                    self.play_selected();
                } else {
                    self.sync_tray();
                }
            }
            Err(e) => {
                self.set_notice(e, true);
            }
        }
    }

    fn pick_local_file(&mut self, lock_after: bool) {
        self.modal_open = true;
        let picked = rfd::FileDialog::new()
            .set_title("Choose local focus audio")
            .add_filter("Audio", &["wav", "mp3", "flac", "ogg", "m4a"])
            .pick_file();
        self.modal_open = false;
        self.suppress_hide_until = Instant::now() + Duration::from_millis(450);

        if let Some(path) = picked {
            self.add_or_select_local_file(path, true, lock_after);
        }
    }

    fn toggle_lock_for_path(&mut self, path: &Path) {
        if self.locked_default_file.as_deref() == Some(path) {
            self.locked_default_file = None;
            self.set_notice("Unlocked default audio", false);
        } else {
            self.locked_default_file = Some(path.to_path_buf());
            self.set_notice("Locked as default audio on launch", false);
        }
        self.missing_locked_file = None;
        self.save_settings();
        self.sync_tray();
    }

    fn play_selected(&mut self) {
        self.has_played = true;
        if self.is_file_mode {
            let Some(path) = self.active_file.clone() else {
                self.pick_local_file(false);
                return;
            };
            let handle = FileStreamHandle::new(path.clone(), self.loop_crossfade);
            let name = self.active_track_title();
            match self.audio.play_file(handle, name) {
                Ok(()) => {
                    self.playing = true;
                    self.missing_locked_file = None;
                    if let Some((msg, true)) = &self.notice {
                        let _ = msg;
                        self.notice = None;
                    }
                }
                Err(e) => {
                    self.playing = false;
                    self.set_notice(e, true);
                    self.sync_tray();
                    return;
                }
            }
        } else {
            self.audio.play_sound(self.current_sound());
            self.playing = true;
        }
        self.elapsed = Duration::ZERO;
        self.resume_timer();
        self.save_settings();
        self.sync_tray();
    }

    fn select_live_sound(&mut self, sound: Sound) {
        if let Some(idx) = Sound::ALL.iter().position(|&s| s == sound) {
            self.active_sound_idx = idx;
            self.is_file_mode = false;
            self.save_settings();
            if self.playing {
                self.play_selected();
            } else {
                self.sync_tray();
            }
        }
    }

    fn select_file_mode(&mut self) {
        if self.active_file.is_none() {
            if let Some(first) = self.local_tracks.first().map(|t| t.path.clone()) {
                self.active_file = Some(first);
            } else {
                self.pick_local_file(false);
                return;
            }
        }
        self.is_file_mode = true;
        self.save_settings();
        if self.playing {
            self.play_selected();
        } else {
            self.sync_tray();
        }
    }

    fn toggle_play(&mut self) {
        if self.playing {
            self.audio.pause();
            self.playing = false;
            self.freeze_timer();
            self.sync_tray();
        } else if self.has_played && self.audio.has_loaded_source() && self.audio.is_file() == self.is_file_mode {
            self.audio.resume();
            self.playing = true;
            self.resume_timer();
            self.sync_tray();
        } else {
            self.play_selected();
        }
    }

    fn step_sound(&mut self, dir: i32) {
        let total = Sound::ALL.len() + usize::from(self.active_file.is_some());
        if total == 0 {
            return;
        }
        let cur = if self.is_file_mode {
            Sound::ALL.len() as i32
        } else {
            self.active_sound_idx as i32
        };
        let next = (cur + dir).rem_euclid(total as i32) as usize;
        if next == Sound::ALL.len() {
            self.is_file_mode = true;
        } else {
            self.is_file_mode = false;
            self.active_sound_idx = next;
        }
        self.save_settings();
        if self.playing {
            self.play_selected();
        } else {
            self.sync_tray();
        }
    }

    fn restart_playback(&mut self) {
        self.has_played = true;
        if self.is_file_mode && self.active_file.is_none() {
            self.pick_local_file(false);
            return;
        }
        if !self.audio.has_loaded_source() || self.audio.is_file() != self.is_file_mode {
            self.play_selected();
            return;
        }
        self.audio.restart();
        self.elapsed = Duration::ZERO;
        self.playing = true;
        self.resume_timer();
        self.sync_tray();
    }

    fn set_volume(&mut self, v: u8) {
        self.audio.set_volume(v.min(100));
        self.save_settings();
        self.sync_tray();
    }

    fn toggle_mute(&mut self) {
        let m = !self.audio.is_muted();
        self.audio.set_muted(m);
        self.save_settings();
        self.sync_tray();
    }

    fn arm_timer(&mut self) {
        if self.timer_minutes == 0 {
            self.timer_remaining = None;
            self.timer_end = None;
            self.audio.set_master_fade(1.0);
            self.sync_tray();
            return;
        }
        let dur = Duration::from_secs(self.timer_minutes as u64 * 60);
        self.timer_remaining = Some(dur);
        if self.playing {
            self.timer_end = Some(Instant::now() + dur);
        } else {
            self.timer_end = None;
        }
        self.audio.set_master_fade(1.0);
        self.sync_tray();
    }

    fn cycle_timer(&mut self) {
        let idx = TIMER_CHOICES
            .iter()
            .position(|&(m, _)| m == self.timer_minutes)
            .unwrap_or(0);
        let next = (idx + 1) % TIMER_CHOICES.len();
        self.timer_minutes = TIMER_CHOICES[next].0;
        self.arm_timer();
        self.save_settings();
    }

    fn freeze_timer(&mut self) {
        if let Some(end) = self.timer_end {
            let rem = end.saturating_duration_since(Instant::now());
            self.timer_remaining = Some(rem);
            self.timer_end = None;
        }
    }

    fn resume_timer(&mut self) {
        if let Some(rem) = self.timer_remaining {
            if rem.as_millis() > 0 {
                self.timer_end = Some(Instant::now() + rem);
            }
        }
    }

    fn reset_sound_controls(&mut self) {
        self.audio.set_volume(DEFAULT_VOLUME);
        self.audio.set_muted(false);
        self.audio.set_depth(DEFAULT_DEPTH);
        self.audio.set_rumble(DEFAULT_RUMBLE);
        self.layer_sound = None;
        self.audio.set_layer(None);
        self.save_settings();
        self.sync_tray();
    }

    fn theme(&self, ctx: &egui::Context) -> Theme {
        match self.theme_mode {
            ThemeMode::Light => LIGHT,
            ThemeMode::Dark => DARK,
            ThemeMode::Auto => {
                if ctx.system_theme() == Some(egui::Theme::Dark) {
                    DARK
                } else {
                    LIGHT
                }
            }
        }
    }

    fn process_external_events(&mut self, ctx: &egui::Context) {
        // 1. IPC commands (e.g. `zenhush --toggle` or GNOME Wayland shortcut)
        let ipc_cmds: Vec<RemoteCmd> = match &self.ipc {
            Some(srv) => {
                let mut v = Vec::new();
                while let Ok(c) = srv.rx.try_recv() {
                    v.push(c);
                }
                v
            }
            None => Vec::new(),
        };
        for cmd in ipc_cmds {
            match cmd {
                RemoteCmd::Toggle => self.toggle_window(ctx),
                RemoteCmd::Show => self.show_window(ctx),
                RemoteCmd::PlayPause => self.toggle_play(),
                RemoteCmd::Play => {
                    if !self.playing {
                        self.toggle_play();
                    }
                }
                RemoteCmd::Pause => {
                    if self.playing {
                        self.toggle_play();
                    }
                }
                RemoteCmd::Next => self.step_sound(1),
                RemoteCmd::Prev => self.step_sound(-1),
                RemoteCmd::VolUp => self.set_volume(self.audio.volume().saturating_add(5)),
                RemoteCmd::VolDown => self.set_volume(self.audio.volume().saturating_sub(5)),
                RemoteCmd::Mute => self.toggle_mute(),
                RemoteCmd::ToggleNotch => self.toggle_notch_mode(ctx),
                RemoteCmd::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            }
        }

        // 2. Global hotkeys (X11 / macOS / Windows)
        for action in self.shortcuts.poll() {
            match action {
                GlobalAction::TogglePopup => self.toggle_window(ctx),
                GlobalAction::TogglePlay => self.toggle_play(),
            }
        }

        // 3. System tray events
        let tray_events: Vec<TrayEvent> = match &self.tray {
            Some(t) => {
                let mut v = Vec::new();
                while let Ok(ev) = t.rx.try_recv() {
                    v.push(ev);
                }
                v
            }
            None => Vec::new(),
        };
        for ev in tray_events {
            match ev {
                TrayEvent::ToggleWindow => self.toggle_window(ctx),
                TrayEvent::TogglePlay => self.toggle_play(),
                TrayEvent::NextSound => self.step_sound(1),
                TrayEvent::PrevSound => self.step_sound(-1),
                TrayEvent::SelectSound(s) => self.select_live_sound(s),
                TrayEvent::SelectFile => self.select_file_mode(),
                TrayEvent::ToggleLockDefault => {
                    if let Some(p) = self.active_file.clone() {
                        self.toggle_lock_for_path(&p);
                    }
                }
                TrayEvent::SetVolume(v) => self.set_volume(v),
                TrayEvent::ToggleMute => self.toggle_mute(),
                TrayEvent::SetTimer(m) => {
                    self.timer_minutes = m;
                    self.arm_timer();
                    self.save_settings();
                }
                TrayEvent::ToggleNotch => self.toggle_notch_mode(ctx),
                TrayEvent::TogglePin => {
                    self.pinned = !self.pinned;
                    self.save_settings();
                    self.sync_tray();
                }
                TrayEvent::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            }
        }
    }

    fn handle_in_app_shortcuts(&mut self, ctx: &egui::Context) {
        // Skip single-letter shortcuts if user is typing in a text input (shortcut fields)
        if ctx.wants_keyboard_input() {
            return;
        }
        let mut actions = Vec::new();
        ctx.input(|i| {
            if i.key_pressed(Key::Space) || i.key_pressed(Key::K) {
                actions.push(0);
            }
            if i.key_pressed(Key::ArrowLeft) || i.key_pressed(Key::OpenBracket) {
                actions.push(1);
            }
            if i.key_pressed(Key::ArrowRight) || i.key_pressed(Key::CloseBracket) {
                actions.push(2);
            }
            if i.key_pressed(Key::ArrowUp) || i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals)
            {
                actions.push(if i.modifiers.shift { 30 } else { 3 });
            }
            if i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::Minus) {
                actions.push(if i.modifiers.shift { 40 } else { 4 });
            }
            if i.key_pressed(Key::M) {
                actions.push(5);
            }
            if i.key_pressed(Key::R) {
                actions.push(6);
            }
            if i.key_pressed(Key::L) {
                actions.push(7);
            }
            if i.key_pressed(Key::N) {
                actions.push(8);
            }
            if i.key_pressed(Key::P) {
                actions.push(9);
            }
            if i.key_pressed(Key::T) {
                actions.push(10);
            }
            if i.key_pressed(Key::S) || i.key_pressed(Key::Comma) {
                actions.push(11);
            }
            if i.key_pressed(Key::Escape) {
                actions.push(12);
            }
            for (idx, k) in [
                Key::Num1,
                Key::Num2,
                Key::Num3,
                Key::Num4,
                Key::Num5,
                Key::Num6,
                Key::Num7,
                Key::Num8,
                Key::Num9,
            ]
            .iter()
            .enumerate()
            {
                if i.key_pressed(*k) {
                    actions.push(100 + idx as i32);
                }
            }
        });

        for a in actions {
            match a {
                0 => self.toggle_play(),
                1 => self.step_sound(-1),
                2 => self.step_sound(1),
                3 => self.set_volume(self.audio.volume().saturating_add(5)),
                30 => self.set_volume(self.audio.volume().saturating_add(1)),
                4 => self.set_volume(self.audio.volume().saturating_sub(5)),
                40 => self.set_volume(self.audio.volume().saturating_sub(1)),
                5 => self.toggle_mute(),
                6 => self.restart_playback(),
                7 => {
                    if let Some(p) = self.active_file.clone() {
                        self.toggle_lock_for_path(&p);
                    }
                }
                8 => self.toggle_notch_mode(ctx),
                9 => {
                    self.pinned = !self.pinned;
                    self.save_settings();
                    self.sync_tray();
                }
                10 => self.cycle_timer(),
                11 => {
                    self.drawer_open = !self.drawer_open;
                    self.apply_window_geometry(ctx);
                }
                12 => {
                    if self.drawer_open {
                        self.drawer_open = false;
                        self.apply_window_geometry(ctx);
                    } else if !self.pinned {
                        self.hide_window(ctx);
                    }
                }
                n if n >= 100 => {
                    let idx = (n - 100) as usize;
                    if idx < Sound::ALL.len() {
                        self.select_live_sound(Sound::ALL[idx]);
                    } else if idx == Sound::ALL.len() && self.active_file.is_some() {
                        self.select_file_mode();
                    }
                }
                _ => {}
            }
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick);
        self.last_tick = now;

        if let Some(until) = self.notice_until {
            if now >= until {
                self.notice = None;
                self.notice_until = None;
            }
        }

        if self.playing {
            self.elapsed += dt;
            if let Some(end) = self.timer_end {
                if now >= end {
                    self.timer_remaining = None;
                    self.timer_end = None;
                    self.timer_minutes = 0;
                    self.audio.pause();
                    self.audio.set_master_fade(1.0);
                    self.playing = false;
                    self.save_settings();
                    self.sync_tray();
                } else {
                    let rem = end.duration_since(now);
                    self.timer_remaining = Some(rem);
                    // Gentle 3-second fade-out before sleep timer stops
                    let secs_left = rem.as_secs_f32();
                    if secs_left <= 3.0 {
                        self.audio.set_master_fade((secs_left / 3.0).clamp(0.0, 1.0));
                    }
                }
            }
        }

        for v in self.audio.drain_rms() {
            if self.rms_history.len() == 64 {
                self.rms_history.pop_front();
            }
            self.rms_history.push_back(v);
        }

        if self.playing && !self.reduce_motion && !self.rms_history.is_empty() {
            let len = self.rms_history.len();
            let mut loudest = 0;
            let mut loudest_v = -1.0f32;
            for i in 0..BAR_COUNT {
                let idx = len.saturating_sub(BAR_COUNT - i);
                let target = self.rms_history.get(idx).copied().unwrap_or(0.14);
                self.bar_heights[i] += (target - self.bar_heights[i]) * 0.42;
                if self.bar_heights[i] > loudest_v {
                    loudest_v = self.bar_heights[i];
                    loudest = i;
                }
            }
            self.bar_accent = loudest;
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) -> bool {
        let mut hovering_file = false;
        let mut dropped: Option<PathBuf> = None;
        ctx.input(|i| {
            hovering_file = !i.raw.hovered_files.is_empty();
            for f in &i.raw.dropped_files {
                if let Some(p) = &f.path {
                    dropped = Some(p.clone());
                    break;
                }
            }
        });
        if let Some(path) = dropped {
            self.add_or_select_local_file(path, true, false);
        }
        hovering_file
    }

    fn render_notch_pill(&mut self, ctx: &egui::Context, theme: Theme) {
        let frame = egui::Frame::default()
            .fill(theme.card)
            .corner_radius(21.0)
            .stroke(Stroke::new(1.0_f32, theme.edge))
            .inner_margin(egui::Margin::symmetric(10, 6));

        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            // Scroll wheel anywhere on the notch pill adjusts volume!
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 1.5 && ui.rect_contains_pointer(ui.max_rect()) {
                let step = if scroll > 0.0 { 3i16 } else { -3i16 };
                let next = (self.audio.volume() as i16 + step).clamp(0, 100) as u8;
                self.set_volume(next);
            }

            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(7.0, 0.0);

                // Mini 6-bar EQ
                let (eq_rect, eq_resp) =
                    ui.allocate_exact_size(Vec2::new(34.0, 20.0), Sense::click());
                if eq_resp.drag_started() {
                    self.dock_anchor = DockAnchor::Free;
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                let p = ui.painter_at(eq_rect);
                for i in 0..6 {
                    let h = (self.bar_heights[i * 3] * 18.0).max(3.0);
                    let x = eq_rect.min.x + i as f32 * 5.6 + 1.0;
                    let y = eq_rect.center().y - h * 0.5;
                    let c = if self.playing && i == (self.bar_accent / 3).min(5) {
                        theme.accent
                    } else if self.playing {
                        theme.bar_strong
                    } else {
                        theme.bar
                    };
                    p.rect_filled(
                        Rect::from_min_size(Pos2::new(x, y), Vec2::new(3.2, h)),
                        CornerRadius::same(2),
                        c,
                    );
                }

                // Title / Timer (draggable too)
                let label_text = if let Some(rem) = self.timer_remaining {
                    if self.playing && rem.as_secs() > 0 {
                        format!(
                            "{} • {}",
                            truncate_chars(&self.display_title(), 12),
                            format_duration(rem.as_secs())
                        )
                    } else {
                        truncate_chars(&self.display_title(), 16)
                    }
                } else {
                    truncate_chars(&self.display_title(), 16)
                };

                let title_w = (ui.available_width() - 68.0).max(40.0);
                let (title_rect, title_resp) =
                    ui.allocate_exact_size(Vec2::new(title_w, 24.0), Sense::click_and_drag());
                if title_resp.drag_started() {
                    self.dock_anchor = DockAnchor::Free;
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                } else if title_resp.clicked() {
                    self.toggle_notch_mode(ctx);
                }
                ui.painter_at(title_rect).text(
                    Pos2::new(title_rect.min.x, title_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    label_text,
                    semibold_font(12.5),
                    theme.text,
                );

                // Mini play/pause button
                let (play_rect, play_resp) =
                    ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
                let pp = ui.painter_at(play_rect);
                pp.circle_filled(
                    play_rect.center(),
                    13.0,
                    if play_resp.is_pointer_button_down_on() {
                        theme.pressed
                    } else {
                        theme.accent
                    },
                );
                let glyph = if self.playing {
                    IconKind::Pause
                } else {
                    IconKind::Play
                };
                if let Some(tex) = self.icons.texture(ctx, glyph, theme.on_accent, 13) {
                    let nudge = if self.playing { 0.0 } else { 1.0 };
                    pp.image(
                        tex.id(),
                        Rect::from_center_size(
                            play_rect.center() + Vec2::new(nudge, 0.0),
                            Vec2::splat(13.0),
                        ),
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                if play_resp
                    .on_hover_text(if self.playing { "Pause (Space)" } else { "Play (Space)" })
                    .clicked()
                {
                    self.toggle_play();
                }

                // Expand to card
                if icon_button(
                    ui,
                    &mut self.icons,
                    IconKind::Maximize,
                    24.0,
                    14,
                    theme.subtext,
                    theme,
                    "Expand card (N)",
                ) {
                    self.toggle_notch_mode(ctx);
                }
            });
        });
    }

    fn render_card(&mut self, ctx: &egui::Context, theme: Theme, hovering_file: bool) {
        let frame = egui::Frame::default()
            .fill(theme.card)
            .corner_radius(22.0)
            .stroke(Stroke::new(
                if hovering_file { 2.0_f32 } else { 1.0_f32 },
                if hovering_file {
                    theme.accent
                } else {
                    theme.edge
                },
            ))
            .inner_margin(egui::Margin::same(12));

        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;

            // --- 1. Top Micro-Bar (Draggable + Pin + Notch + Hide) ---
            ui.horizontal(|ui| {
                ui.set_height(24.0);
                let brand_color = if self.playing {
                    theme.accent_text
                } else {
                    theme.subtext
                };
                let pill_text = if self.is_current_locked_default() {
                    "ZENHUSH • LOCKED"
                } else if self.playing {
                    "ZENHUSH • ACTIVE"
                } else {
                    "ZENHUSH"
                };

                let drag_w = (ui.available_width() - 80.0).max(60.0);
                let (drag_rect, drag_resp) =
                    ui.allocate_exact_size(Vec2::new(drag_w, 22.0), Sense::click_and_drag());
                if drag_resp.drag_started() {
                    self.dock_anchor = DockAnchor::Free;
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                ui.painter_at(drag_rect).text(
                    Pos2::new(drag_rect.min.x + 2.0, drag_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    pill_text,
                    semibold_font(10.5),
                    brand_color,
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(4.0, 0.0);
                    if self.tray.is_some()
                        && icon_button(
                            ui,
                            &mut self.icons,
                            IconKind::Close,
                            22.0,
                            13,
                            theme.subtext,
                            theme,
                            "Hide to top bar (Esc)",
                        )
                    {
                        self.hide_window(ctx);
                    }
                    if icon_button(
                        ui,
                        &mut self.icons,
                        IconKind::Minimize,
                        22.0,
                        13,
                        theme.subtext,
                        theme,
                        "Notch Pill mode (N)",
                    ) {
                        self.toggle_notch_mode(ctx);
                    }
                    if icon_button(
                        ui,
                        &mut self.icons,
                        IconKind::Pin,
                        22.0,
                        13,
                        if self.pinned {
                            theme.accent_text
                        } else {
                            theme.subtext
                        },
                        theme,
                        if self.pinned {
                            "Pinned open (P)"
                        } else {
                            "Pin open (P)"
                        },
                    ) {
                        self.pinned = !self.pinned;
                        self.save_settings();
                        self.sync_tray();
                    }
                });
            });

            ui.add_space(6.0);

            // --- 2. Visualizer Card (92px, 18 smooth bars, drop target) ---
            let eq_w = ui.available_width();
            let (eq_rect, eq_resp) = ui.allocate_exact_size(Vec2::new(eq_w, 92.0), Sense::click());
            if eq_resp.double_clicked() {
                self.toggle_play();
            }
            let p = ui.painter_at(eq_rect);
            p.rect_filled(eq_rect, CornerRadius::same(12), theme.inset);

            if hovering_file {
                p.text(
                    eq_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Drop audio file to play & loop",
                    semibold_font(13.0),
                    theme.accent_text,
                );
            } else {
                let margin = 12.0;
                let top = 10.0;
                let bottom = 26.0;
                let bar_h = eq_rect.height() - top - bottom - 6.0;
                let slot = (eq_rect.width() - 2.0 * margin) / BAR_COUNT as f32;
                let bw = (slot * 0.56).max(3.0);

                for i in 0..BAR_COUNT {
                    let h = (self.bar_heights[i] * bar_h).max(3.0);
                    let x = eq_rect.min.x + margin + i as f32 * slot + (slot - bw) * 0.5;
                    let y = eq_rect.min.y + top + bar_h - h;
                    let color = if i == self.bar_accent && self.playing {
                        theme.accent
                    } else if self.bar_heights[i] > 0.62 {
                        theme.bar_strong
                    } else {
                        theme.bar
                    };
                    p.rect_filled(
                        Rect::from_min_size(Pos2::new(x, y), Vec2::new(bw, h)),
                        CornerRadius::same((bw * 0.5).max(1.0) as u8),
                        color,
                    );
                }

                // Bottom-left: elapsed; bottom-right: status/countdown
                p.text(
                    Pos2::new(eq_rect.min.x + margin, eq_rect.max.y - 8.0),
                    egui::Align2::LEFT_BOTTOM,
                    format_duration(self.elapsed.as_secs()),
                    egui::FontId::proportional(11.5),
                    theme.subtext,
                );
                let badge = self.status_badge();
                p.text(
                    Pos2::new(eq_rect.max.x - margin, eq_rect.max.y - 8.0),
                    egui::Align2::RIGHT_BOTTOM,
                    badge,
                    semibold_font(11.5),
                    theme.accent_text,
                );
            }

            // Missing locked file recovery or transient notice
            if let Some(missing) = self.missing_locked_file.clone() {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "Missing: {}",
                            truncate_chars(
                                &missing
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_default(),
                                18
                            )
                        ))
                        .size(11.5)
                        .color(theme.accent_text),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Unlock").size(11.0).color(theme.subtext),
                                )
                                .frame(false),
                            )
                            .clicked()
                        {
                            self.locked_default_file = None;
                            self.missing_locked_file = None;
                            self.is_file_mode = false;
                            self.notice = None;
                            self.save_settings();
                        }
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Relocate…")
                                        .size(11.0)
                                        .color(theme.accent_text),
                                )
                                .frame(false),
                            )
                            .clicked()
                        {
                            self.pick_local_file(true);
                        }
                    });
                });
            } else if let Some((msg, is_err)) = &self.notice {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(msg)
                        .size(11.5)
                        .color(if *is_err {
                            theme.accent_text
                        } else {
                            theme.subtext
                        }),
                );
            }

            ui.add_space(10.0);

            // --- 3. Sound Header (< Title + Subtitle >) ---
            ui.horizontal(|ui| {
                if icon_button(
                    ui,
                    &mut self.icons,
                    IconKind::ChevronLeft,
                    30.0,
                    17,
                    theme.subtext,
                    theme,
                    "Previous sound (Left)",
                ) {
                    self.step_sound(-1);
                }

                let mid_w = (ui.available_width() - 34.0).max(20.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(mid_w, 36.0),
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(0.0, 1.0);
                        ui.label(
                            RichText::new(self.display_title())
                                .font(bold_font(15.5))
                                .color(theme.text),
                        );
                        ui.label(
                            RichText::new(self.display_subtitle())
                                .size(12.0)
                                .color(theme.subtext),
                        );
                    },
                );

                if icon_button(
                    ui,
                    &mut self.icons,
                    IconKind::ChevronRight,
                    30.0,
                    17,
                    theme.subtext,
                    theme,
                    "Next sound (Right)",
                ) {
                    self.step_sound(1);
                }
            });

            ui.add_space(8.0);

            // --- 4. Quick-Jump Sound Pills (1-click switching!) ---
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                for &sound in &Sound::ALL {
                    let selected = !self.is_file_mode && self.current_sound() == sound;
                    if pill_chip(ui, sound.short_label(), selected, theme) {
                        self.select_live_sound(sound);
                    }
                }
                let file_chip_label = if self.locked_default_file.is_some() {
                    "🔒 File"
                } else {
                    "File"
                };
                if pill_chip(ui, file_chip_label, self.is_file_mode, theme) {
                    if self.active_file.is_none() && self.local_tracks.is_empty() {
                        self.drawer_open = true;
                        self.drawer_tab = DrawerTab::Files;
                        self.apply_window_geometry(ctx);
                        self.pick_local_file(false);
                    } else {
                        self.select_file_mode();
                    }
                }
            });

            ui.add_space(10.0);

            // --- 5. Hero Transport Row ---
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(14.0, 0.0);
                let row_w = 34.0 + 14.0 + 54.0 + 14.0 + 58.0 + 14.0 + 34.0;
                ui.add_space(((ui.available_width() - row_w) * 0.5).max(0.0));

                if icon_button(
                    ui,
                    &mut self.icons,
                    IconKind::Restart,
                    34.0,
                    18,
                    theme.subtext,
                    theme,
                    "Restart (R)",
                ) {
                    self.restart_playback();
                }

                // Play/Pause hero circle (54px)
                let (play_rect, play_resp) =
                    ui.allocate_exact_size(Vec2::splat(54.0), Sense::click());
                let pp = ui.painter_at(play_rect);
                let fill = if play_resp.is_pointer_button_down_on() {
                    theme.pressed
                } else {
                    theme.accent
                };
                pp.circle_filled(play_rect.center(), 26.5, fill);
                if play_resp.has_focus() {
                    pp.circle_stroke(play_rect.center(), 24.5, Stroke::new(2.0_f32, theme.accent_text));
                }
                let glyph = if self.playing {
                    IconKind::Pause
                } else {
                    IconKind::Play
                };
                if let Some(tex) = self.icons.texture(ctx, glyph, theme.on_accent, 22) {
                    let nudge = if self.playing { 0.0 } else { 1.5 };
                    pp.image(
                        tex.id(),
                        Rect::from_center_size(
                            play_rect.center() + Vec2::new(nudge, 0.0),
                            Vec2::splat(22.0),
                        ),
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                if play_resp
                    .on_hover_text(if self.playing {
                        "Pause (Space)"
                    } else {
                        "Play (Space)"
                    })
                    .clicked()
                {
                    self.toggle_play();
                }

                // Quick Timer Cycle Pill
                let timer_txt = if self.timer_minutes == 0 {
                    "Timer".to_string()
                } else {
                    format!("{}m", self.timer_minutes)
                };
                let active_timer = self.timer_minutes > 0;
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(timer_txt).size(11.5).color(if active_timer {
                                theme.on_accent
                            } else {
                                theme.subtext
                            }),
                        )
                        .fill(if active_timer {
                            theme.accent
                        } else {
                            theme.field
                        })
                        .corner_radius(CornerRadius::same(14))
                        .min_size(Vec2::new(58.0, 30.0)),
                    )
                    .on_hover_text("Cycle sleep/focus timer (T)")
                    .clicked()
                {
                    self.cycle_timer();
                }

                // Drawer Toggle
                if icon_button(
                    ui,
                    &mut self.icons,
                    IconKind::Sliders,
                    34.0,
                    18,
                    if self.drawer_open {
                        theme.accent_text
                    } else {
                        theme.subtext
                    },
                    theme,
                    "Files, Mix & Shortcuts (S)",
                ) {
                    self.drawer_open = !self.drawer_open;
                    self.apply_window_geometry(ctx);
                }
            });

            ui.add_space(10.0);

            // --- 6. Always-Visible Volume + Quick Lock Strip ---
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 0.0);
                let muted = self.audio.is_muted();
                if icon_button(
                    ui,
                    &mut self.icons,
                    if muted || self.audio.volume() == 0 {
                        IconKind::Mute
                    } else {
                        IconKind::Volume
                    },
                    26.0,
                    16,
                    if muted { theme.accent_text } else { theme.subtext },
                    theme,
                    "Mute / Unmute (M)",
                ) {
                    self.toggle_mute();
                }

                let lock_btn_w = if self.is_file_mode && self.active_file.is_some() {
                    30.0
                } else {
                    0.0
                };
                let slider_w = (ui.available_width() - 36.0 - lock_btn_w).max(60.0);
                let mut vol_val = self.audio.volume() as f32;
                let resp = ui.add_sized(
                    [slider_w, 20.0],
                    egui::Slider::new(&mut vol_val, 0.0..=100.0)
                        .show_value(false)
                        .trailing_fill(true),
                );
                // Scroll wheel on volume slider
                if resp.hovered() {
                    let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                    if scroll.abs() > 1.0 {
                        let step = if scroll > 0.0 { 3.0 } else { -3.0 };
                        vol_val = (vol_val + step).clamp(0.0, 100.0);
                        self.set_volume(vol_val as u8);
                    }
                }
                if resp.changed() {
                    self.set_volume(vol_val as u8);
                }

                let vol_txt = if muted {
                    "Mute".to_string()
                } else {
                    format!("{}%", self.audio.volume())
                };
                ui.allocate_ui_with_layout(
                    Vec2::new(30.0, 20.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        ui.label(RichText::new(vol_txt).size(11.5).color(theme.subtext));
                    },
                );

                // Quick Lock-as-Default button right on the main bar when a file is selected!
                if self.is_file_mode {
                    if let Some(path) = self.active_file.clone() {
                        let locked = self.locked_default_file.as_ref() == Some(&path);
                        if icon_button(
                            ui,
                            &mut self.icons,
                            if locked { IconKind::Lock } else { IconKind::Unlock },
                            26.0,
                            15,
                            if locked { theme.accent_text } else { theme.subtext },
                            theme,
                            if locked {
                                "Locked as default audio (L to unlock)"
                            } else {
                                "Lock this file as default audio (L)"
                            },
                        ) {
                            self.toggle_lock_for_path(&path);
                        }
                    }
                }
            });

            // --- Quit link when no system tray is available ---
            if self.tray.is_none() && !self.drawer_open {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space((ui.available_width() - 90.0).max(0.0) * 0.5);
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Quit Zenhush")
                                    .size(11.5)
                                    .color(theme.accent_text),
                            )
                            .frame(false)
                            .min_size(Vec2::new(90.0, 20.0)),
                        )
                        .clicked()
                    {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            }

            // --- 7. Tabbed Drawer ---
            if self.drawer_open {
                ui.add_space(10.0);
                ui.painter().line_segment(
                    [
                        Pos2::new(ui.min_rect().min.x, ui.cursor().min.y),
                        Pos2::new(ui.min_rect().min.x + ui.available_width(), ui.cursor().min.y),
                    ],
                    Stroke::new(1.0_f32, theme.edge),
                );
                ui.add_space(8.0);

                // Tab selector row
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(6.0, 0.0);
                    let tab_w = (ui.available_width() - 12.0) / 3.0;
                    for (tab, lbl) in [
                        (DrawerTab::Files, "Local Audio"),
                        (DrawerTab::MixTimer, "Mix & Timer"),
                        (DrawerTab::BarKeys, "Bar & Keys"),
                    ] {
                        let active = self.drawer_tab == tab;
                        if ui
                            .add_sized(
                                [tab_w, 26.0],
                                egui::Button::new(RichText::new(lbl).size(11.5).color(if active {
                                    theme.on_accent
                                } else {
                                    theme.subtext
                                }))
                                .fill(if active { theme.accent } else { theme.field })
                                .corner_radius(CornerRadius::same(10)),
                            )
                            .clicked()
                        {
                            self.drawer_tab = tab;
                        }
                    }
                });

                ui.add_space(10.0);
                match self.drawer_tab {
                    DrawerTab::Files => self.render_tab_files(ui, theme),
                    DrawerTab::MixTimer => self.render_tab_mix(ui, theme),
                    DrawerTab::BarKeys => self.render_tab_barkeys(ui, ctx, theme),
                }
            }
        });
    }

    fn render_tab_files(&mut self, ui: &mut egui::Ui, theme: Theme) {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 8.0);

        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("+ Add Audio File")
                            .size(12.0)
                            .color(theme.on_accent),
                    )
                    .fill(theme.accent)
                    .corner_radius(CornerRadius::same(10))
                    .min_size(Vec2::new(122.0, 28.0)),
                )
                .clicked()
            {
                self.pick_local_file(false);
            }
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("Add & Lock Default")
                            .size(11.5)
                            .color(theme.accent_text),
                    )
                    .fill(theme.field)
                    .corner_radius(CornerRadius::same(10))
                    .min_size(Vec2::new(124.0, 28.0)),
                )
                .on_hover_text("Pick a file and lock it as Zenhush's default startup audio")
                .clicked()
            {
                self.pick_local_file(true);
            }
        });

        // Saved tracks list
        let mut play_path: Option<PathBuf> = None;
        let mut lock_path: Option<PathBuf> = None;
        let mut remove_path: Option<PathBuf> = None;

        egui::ScrollArea::vertical()
            .max_height(104.0)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                if self.local_tracks.is_empty() {
                    ui.label(
                        RichText::new("No local tracks yet — add or drop wav, mp3, flac, ogg, m4a")
                            .size(11.5)
                            .color(theme.subtext),
                    );
                }
                for track in &self.local_tracks {
                    let is_active =
                        self.is_file_mode && self.active_file.as_ref() == Some(&track.path);
                    let is_locked = self.locked_default_file.as_ref() == Some(&track.path);
                    let row_bg = if is_active {
                        theme.accent_soft
                    } else {
                        theme.field
                    };

                    egui::Frame::default()
                        .fill(row_bg)
                        .corner_radius(8.0)
                        .inner_margin(egui::Margin::symmetric(8, 5))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let dur = track
                                    .duration_secs
                                    .map(|d| format!(" ({})", format_duration(d)))
                                    .unwrap_or_default();
                                let label = format!("{}{}", truncate_chars(&track.title, 20), dur);
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(label).size(12.0).color(if is_active {
                                                theme.accent_text
                                            } else {
                                                theme.text
                                            }),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                                {
                                    play_path = Some(track.path.clone());
                                }

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if icon_button(
                                            ui,
                                            &mut self.icons,
                                            IconKind::Trash,
                                            22.0,
                                            13,
                                            theme.subtext,
                                            theme,
                                            "Remove from list",
                                        ) {
                                            remove_path = Some(track.path.clone());
                                        }
                                        if icon_button(
                                            ui,
                                            &mut self.icons,
                                            if is_locked {
                                                IconKind::Lock
                                            } else {
                                                IconKind::Unlock
                                            },
                                            22.0,
                                            13,
                                            if is_locked {
                                                theme.accent_text
                                            } else {
                                                theme.subtext
                                            },
                                            theme,
                                            if is_locked {
                                                "Locked as default audio"
                                            } else {
                                                "Lock as default audio"
                                            },
                                        ) {
                                            lock_path = Some(track.path.clone());
                                        }
                                    },
                                );
                            });
                        });
                }
            });

        if let Some(p) = play_path {
            self.add_or_select_local_file(p, true, false);
        }
        if let Some(p) = lock_path {
            self.toggle_lock_for_path(&p);
        }
        if let Some(p) = remove_path {
            self.local_tracks.retain(|t| t.path != p);
            if self.locked_default_file.as_ref() == Some(&p) {
                self.locked_default_file = None;
            }
            if self.active_file.as_ref() == Some(&p) {
                self.active_file = self.local_tracks.first().map(|t| t.path.clone());
                if self.active_file.is_none() {
                    self.is_file_mode = false;
                    if self.playing {
                        self.play_selected();
                    }
                }
            }
            self.save_settings();
            self.sync_tray();
        }

        // File Clarity EQ + Loop Seam
        ui.horizontal(|ui| {
            ui.label(RichText::new("Tone").size(12.0).color(theme.subtext));
            ui.add_space(6.0);
            for tone in FileTone::ALL {
                if pill_chip(ui, tone.label(), self.file_tone == tone, theme) {
                    self.file_tone = tone;
                    self.audio.set_file_tone(tone);
                    self.save_settings();
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Loop seam").size(12.0).color(theme.subtext));
            ui.add_space(6.0);
            if pill_chip(ui, "Crossfade", self.loop_crossfade, theme) {
                self.loop_crossfade = true;
                self.audio.set_crossfade(true);
                self.save_settings();
            }
            if pill_chip(ui, "Gapless", !self.loop_crossfade, theme) {
                self.loop_crossfade = false;
                self.audio.set_crossfade(false);
                self.save_settings();
            }
        });
    }

    fn render_tab_mix(&mut self, ui: &mut egui::Ui, theme: Theme) {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 8.0);

        ui.horizontal(|ui| {
            ui.label(
                RichText::new("SOUND SHAPING")
                    .font(semibold_font(10.5))
                    .color(theme.subtext),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("Reset").size(11.5).color(theme.accent_text),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    self.reset_sound_controls();
                }
            });
        });

        let mut dep = self.audio.depth() as f32;
        ui.horizontal(|ui| {
            ui.allocate_ui(Vec2::new(52.0, 18.0), |ui| {
                ui.label(RichText::new("Depth").size(12.5).color(theme.subtext));
            });
            let w = (ui.available_width() - 32.0).max(60.0);
            if ui
                .add_sized(
                    [w, 18.0],
                    egui::Slider::new(&mut dep, 0.0..=100.0)
                        .show_value(false)
                        .trailing_fill(true),
                )
                .on_hover_text("Higher cuts bright frequencies for warmer focus")
                .changed()
            {
                self.audio.set_depth(dep as u8);
                self.save_settings();
            }
            ui.label(
                RichText::new(format!("{}", self.audio.depth()))
                    .size(12.0)
                    .color(theme.subtext),
            );
        });

        let mut rum = self.audio.rumble() as f32;
        ui.horizontal(|ui| {
            ui.allocate_ui(Vec2::new(52.0, 18.0), |ui| {
                ui.label(RichText::new("Rumble").size(12.5).color(theme.subtext));
            });
            let w = (ui.available_width() - 32.0).max(60.0);
            if ui
                .add_sized(
                    [w, 18.0],
                    egui::Slider::new(&mut rum, 0.0..=100.0)
                        .show_value(false)
                        .trailing_fill(true),
                )
                .on_hover_text("Sub-bass warmth boost")
                .changed()
            {
                self.audio.set_rumble(rum as u8);
                self.save_settings();
            }
            ui.label(
                RichText::new(format!("{}", self.audio.rumble()))
                    .size(12.0)
                    .color(theme.subtext),
            );
        });

        // Atmosphere Layer
        ui.add_space(2.0);
        ui.label(
            RichText::new("ATMOSPHERE LAYER")
                .font(semibold_font(10.5))
                .color(theme.subtext),
        );
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            if pill_chip(ui, "Off", self.layer_sound.is_none(), theme) {
                self.layer_sound = None;
                self.audio.set_layer(None);
                self.save_settings();
            }
            for &ls in &Sound::ATMOSPHERE_LAYERS {
                if pill_chip(ui, ls.short_label(), self.layer_sound == Some(ls), theme) {
                    self.layer_sound = Some(ls);
                    self.audio.set_layer(Some(ls));
                    self.save_settings();
                }
            }
        });
        if self.layer_sound.is_some() {
            let mut mix = self.layer_mix as f32;
            ui.horizontal(|ui| {
                ui.allocate_ui(Vec2::new(52.0, 18.0), |ui| {
                    ui.label(RichText::new("Blend").size(12.5).color(theme.subtext));
                });
                let w = (ui.available_width() - 32.0).max(60.0);
                if ui
                    .add_sized(
                        [w, 18.0],
                        egui::Slider::new(&mut mix, 0.0..=100.0)
                            .show_value(false)
                            .trailing_fill(true),
                    )
                    .changed()
                {
                    self.layer_mix = mix as u8;
                    self.audio.set_layer_mix(self.layer_mix);
                    self.save_settings();
                }
                ui.label(
                    RichText::new(format!("{}%", self.layer_mix))
                        .size(12.0)
                        .color(theme.subtext),
                );
            });
        }

        // Sleep / Focus Timer
        ui.add_space(2.0);
        ui.label(
            RichText::new("FOCUS / SLEEP TIMER (FADES OUT)")
                .font(semibold_font(10.5))
                .color(theme.subtext),
        );
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            for (mins, lbl) in TIMER_CHOICES {
                if pill_chip(ui, lbl, self.timer_minutes == mins, theme) {
                    self.timer_minutes = mins;
                    self.arm_timer();
                    self.save_settings();
                }
            }
        });
    }

    fn render_tab_barkeys(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, theme: Theme) {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 7.0);

        ui.horizontal(|ui| {
            ui.label(RichText::new("Dock").size(12.0).color(theme.subtext));
            ui.add_space(4.0);
            for anchor in DockAnchor::ALL {
                if pill_chip(ui, anchor.label(), self.dock_anchor == anchor, theme) {
                    self.dock_anchor = anchor;
                    self.anchor_applied = false;
                    self.apply_window_geometry(ctx);
                    self.save_settings();
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label(RichText::new("Theme").size(12.0).color(theme.subtext));
            ui.add_space(4.0);
            for mode in ThemeMode::ALL {
                if pill_chip(ui, mode.label(), self.theme_mode == mode, theme) {
                    self.theme_mode = mode;
                    self.save_settings();
                }
            }
        });

        ui.horizontal(|ui| {
            if pill_chip(ui, "Auto-play on launch", self.auto_play_on_launch, theme) {
                self.auto_play_on_launch = !self.auto_play_on_launch;
                self.save_settings();
            }
            if pill_chip(ui, "Start in tray", self.start_in_background, theme) {
                self.start_in_background = !self.start_in_background;
                self.save_settings();
            }
            if pill_chip(ui, "Reduce motion", self.reduce_motion, theme) {
                self.reduce_motion = !self.reduce_motion;
                self.save_settings();
            }
        });

        #[cfg(target_os = "linux")]
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            if pill_chip(ui, "Launch at login", self.autostart_enabled, theme) {
                let target = !self.autostart_enabled;
                match shortcuts::set_autostart(target) {
                    Ok(()) => {
                        self.autostart_enabled = target;
                        self.set_notice(
                            if target {
                                "Autostart enabled (starts in top bar)"
                            } else {
                                "Autostart removed"
                            },
                            false,
                        );
                    }
                    Err(e) => self.set_notice(e, true),
                }
            }
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("Install Launcher")
                            .size(11.5)
                            .color(theme.text),
                    )
                    .fill(theme.field)
                    .corner_radius(CornerRadius::same(10)),
                )
                .on_hover_text("Install zenhush to ~/.local/bin and your Linux App Launcher")
                .clicked()
            {
                match shortcuts::install_to_user_local() {
                    Ok(_) => self.set_notice("Installed to ~/.local/bin & App Launcher", false),
                    Err(e) => self.set_notice(e, true),
                }
            }
            if shortcuts::is_gnome()
                && ui
                    .add(
                        egui::Button::new(
                            RichText::new("Bind GNOME Shortcut")
                                .size(11.5)
                                .color(theme.on_accent),
                        )
                        .fill(theme.accent)
                        .corner_radius(CornerRadius::same(10)),
                    )
                    .on_hover_text("Registers toggle & play/pause shortcuts in GNOME")
                    .clicked()
            {
                match shortcuts::register_gnome_shortcuts(
                    &self.global_shortcut_toggle,
                    &self.global_shortcut_play,
                ) {
                    Ok(()) => self.set_notice(
                        format!(
                            "GNOME bound: {} & {}",
                            self.global_shortcut_toggle, self.global_shortcut_play
                        ),
                        false,
                    ),
                    Err(e) => self.set_notice(e, true),
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label(RichText::new("Hotkey").size(11.5).color(theme.subtext));
            for preset in ["Alt+Shift+Z", "Ctrl+Shift+Z", "Super+Shift+Z"] {
                if pill_chip(ui, preset, self.global_shortcut_toggle == preset, theme) {
                    self.global_shortcut_toggle = preset.to_string();
                    self.shortcuts
                        .apply(&self.global_shortcut_toggle, &self.global_shortcut_play);
                    self.save_settings();
                }
            }
        });

        ui.label(
            RichText::new(format!(
                "Play: {}  |  {}",
                self.global_shortcut_play, self.shortcuts.status_note
            ))
            .size(11.0)
            .color(theme.subtext),
        );
        ui.label(
            RichText::new(
                "Keys: Space Play • ←/→ Sound • ↑/↓ Vol • M Mute • L Lock • N Notch • P Pin • Esc Hide",
            )
            .size(10.5)
            .color(theme.subtext),
        );
        ui.label(
            RichText::new(format!("Output: {}", truncate_chars(self.audio.device_name(), 34)))
                .size(10.5)
                .color(theme.subtext),
        );
    }
}

impl eframe::App for ZenhushApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.shortcuts.bind_egui_ctx(ctx);
        self.process_external_events(ctx);
        self.audio.maintain();
        self.tick();

        // Save position if dragged in Free mode (debounced so it persists safely)
        if self.dock_anchor == DockAnchor::Free {
            if let Some(pos) = ctx.input(|i| i.viewport().outer_rect.map(|r| [r.min.x, r.min.y])) {
                if self.saved_pos != Some(pos) {
                    self.saved_pos = Some(pos);
                    self.last_pos_change = Some(Instant::now());
                }
            }
            if let Some(changed_at) = self.last_pos_change {
                if changed_at.elapsed() >= Duration::from_millis(600) {
                    self.last_pos_change = None;
                    self.save_settings();
                }
            }
        }

        // Auto-hide on focus loss unless pinned, in Notch Pill mode, or picking a file
        if self.visible {
            let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
            if focused {
                self.was_focused = true;
            }
            let allow_hide = !self.pinned
                && !self.modal_open
                && self.window_mode == WindowMode::Card
                && Instant::now() >= self.suppress_hide_until
                && self.tray.is_some();
            if allow_hide && self.was_focused && !focused {
                self.hide_window(ctx);
            }
        }

        self.sync_tray();

        if !self.visible {
            // Sleep completely when hidden & paused (0% CPU): Tray, IPC, and
            // global hotkeys wake egui directly via ctx.request_repaint().
            if self.playing || self.timer_end.is_some() {
                ctx.request_repaint_after(Duration::from_millis(1000));
            }
            return;
        }

        self.apply_window_geometry(ctx);
        self.handle_in_app_shortcuts(ctx);
        let hovering_file = self.handle_dropped_files(ctx);

        let theme = self.theme(ctx);
        let is_dark = self.theme_mode == ThemeMode::Dark
            || (self.theme_mode == ThemeMode::Auto
                && ctx.system_theme() == Some(egui::Theme::Dark));
        let mut visuals = if is_dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        visuals.window_fill = theme.card;
        visuals.panel_fill = theme.card;
        visuals.widgets.noninteractive.bg_fill = theme.card;
        visuals.widgets.inactive.bg_fill = theme.field;
        visuals.widgets.hovered.bg_fill = theme.hover;
        visuals.widgets.active.bg_fill = theme.field;
        visuals.selection.bg_fill = theme.accent;
        visuals.selection.stroke = Stroke::new(1.0_f32, theme.accent);
        ctx.set_visuals(visuals);

        if self.playing || self.timer_end.is_some() || self.notice.is_some() {
            ctx.request_repaint_after(Duration::from_millis(75));
        } else {
            ctx.request_repaint_after(Duration::from_millis(250));
        }

        match self.window_mode {
            WindowMode::NotchPill => self.render_notch_pill(ctx, theme),
            WindowMode::Card => self.render_card(ctx, theme, hovering_file),
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_settings();
    }
}

fn pill_chip(ui: &mut egui::Ui, label: &str, selected: bool, theme: Theme) -> bool {
    ui.add(
        egui::Button::new(
            RichText::new(label)
                .size(11.5)
                .color(if selected { theme.on_accent } else { theme.text }),
        )
        .fill(if selected { theme.accent } else { theme.field })
        .corner_radius(CornerRadius::same(11))
        .min_size(Vec2::new(0.0, 24.0)),
    )
    .clicked()
}

fn icon_button(
    ui: &mut egui::Ui,
    icons: &mut IconCache,
    kind: IconKind,
    button_px: f32,
    glyph_px: u32,
    color: Color32,
    theme: Theme,
    tooltip: &str,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(button_px), Sense::click());
    let p = ui.painter_at(rect);
    let r = (button_px * 0.5) as u8;
    if resp.is_pointer_button_down_on() {
        p.rect_filled(rect, CornerRadius::same(r), theme.field);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(r), theme.hover);
    }
    if resp.has_focus() {
        p.rect_stroke(
            rect,
            CornerRadius::same(r),
            Stroke::new(2.0_f32, theme.accent_text),
            egui::StrokeKind::Inside,
        );
    }
    if let Some(tex) = icons.texture(ui.ctx(), kind, color, glyph_px) {
        let g = glyph_px as f32;
        p.image(
            tex.id(),
            Rect::from_center_size(rect.center(), Vec2::splat(g)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    resp.on_hover_text(tooltip).clicked()
}
