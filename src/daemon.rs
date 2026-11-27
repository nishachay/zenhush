//! Ultra-lightweight background Core (`--daemon` / `--background`).
//!
//! Owns `AudioEngine`, `MprisServer` (`org.zenhush.Control`), optional fallback
//! `Tray`, `ShortcutManager`, `IpcServer`, and `BridgeServer` with zero
//! OpenGL/Mesa/winit allocations (~15 MB RSS, 0% idle CPU).
//!
//! On GNOME Linux with the native Top-Bar Popover extension active, Zenhush
//! lives completely inside the GNOME top bar — no floating `--ui` window is
//! spawned at all. On other desktops (macOS/Windows/KDE) or before extension
//! activation, falls back seamlessly to the lightweight on-demand `--ui` card.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::audio::AudioEngine;
use crate::bridge::{BridgeServer, CoreToUi, UiCmd, UiSnapshot};
use crate::filemode::{self, FileStreamHandle};
use crate::gnome_ext;
use crate::ipc::{IpcServer, RemoteCmd};
use crate::mpris::{MprisCmd, MprisServer, MprisState};
use crate::settings::{self, DockAnchor, LocalTrack, Settings, ThemeMode, WindowMode};
use crate::shortcuts::{self, GlobalAction, ShortcutManager};
use crate::synth::{FileTone, Sound, DEFAULT_DEPTH, DEFAULT_RUMBLE, DEFAULT_VOLUME};
use crate::tray::{Tray, TrayEvent, TrayState};

const BAR_COUNT: usize = 8;
const IDLE_BARS: [f32; BAR_COUNT] = [0.35, 0.60, 0.80, 0.55, 0.90, 0.65, 0.45, 0.30];
const TIMER_CYCLE: [u32; 5] = [0, 25, 45, 60, 90];

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn format_duration(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

pub fn run_daemon(open_ui_on_start: bool) {
    let Some(_daemon_lock) = crate::ipc::InstanceLock::try_acquire("zenhush-daemon") else {
        if open_ui_on_start {
            let _ = crate::ipc::wait_and_send_to_existing(RemoteCmd::Show, 1500);
        }
        return;
    };

    let main_thread = Arc::new(thread::current());
    // Bind IPC + UI bridge within ~2ms so `--ui` and rapid double-clicks never race
    let ipc = IpcServer::start(Some(main_thread.clone()));
    let bridge = BridgeServer::start(Some(main_thread.clone()));
    let mut settings = settings::load();

    // Auto-install/upgrade native GNOME Top-Bar Popover extension unless user opted out
    if shortcuts::is_gnome() && !settings.gnome_pill_opted_out {
        gnome_ext::ensure_installed_and_enabled();
    }
    let gnome_popover_active = gnome_ext::is_active();

    let mut audio = AudioEngine::new();
    audio.set_volume(settings.volume);
    audio.set_muted(settings.muted);
    audio.set_depth(settings.depth);
    audio.set_rumble(settings.rumble);
    audio.set_layer(settings.layer_sound);
    audio.set_layer_mix(settings.layer_mix);
    audio.set_file_tone(settings.file_tone);

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

    let mut is_file_mode = false;
    let mut active_file = settings.active_file.clone();
    let mut missing_locked_file = None;
    let mut notice: Option<(String, bool)> = None;

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

    // Avoid duplicate top-bar icons: only create ksni tray when GNOME popover isn't active
    let tray = if gnome_popover_active {
        None
    } else {
        Tray::new(Some(main_thread.clone()))
    };
    let shortcuts = ShortcutManager::new(
        &settings.global_shortcut_toggle,
        &settings.global_shortcut_play,
        Some(main_thread.clone()),
    );
    let (mpris_tx, mpris_rx): (_, Receiver<MprisCmd>) = channel();
    let mpris = MprisServer::start(mpris_tx, Some(main_thread.clone()));
    let (file_pick_tx, file_pick_rx) = channel::<(PathBuf, bool)>();

    let mut core = Core {
        main_thread,
        audio,
        tray,
        ipc,
        bridge,
        mpris,
        mpris_rx,
        file_pick_tx,
        file_pick_rx,
        picking_file: Arc::new(AtomicBool::new(false)),
        shortcuts,
        ui_child: None,
        ui_was_connected: false,
        running: true,
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
        gnome_pill_opted_out: settings.gnome_pill_opted_out,
        autostart_enabled: shortcuts::is_autostart_enabled(),
        gnome_pill_installed: gnome_ext::is_installed(),
        gnome_popover_active,
        global_shortcut_toggle: settings.global_shortcut_toggle,
        global_shortcut_play: settings.global_shortcut_play,
        rms_history: VecDeque::with_capacity(32),
        bar_heights: IDLE_BARS,
        notice,
        notice_until: None,
        missing_locked_file,
        last_pushed_snap: None,
        last_rms_push: Instant::now(),
    };

    if core.timer_minutes > 0 {
        core.arm_timer();
    }
    if core.auto_play_on_launch {
        core.play_selected();
    }
    core.sync_outputs();

    if open_ui_on_start {
        if core.gnome_popover_active {
            // Give GNOME extension ~60ms to spot our D-Bus name, then pop open
            thread::sleep(Duration::from_millis(60));
            core.toggle_or_open_primary(None);
        } else {
            core.open_ui(None);
        }
    }

    while core.running {
        core.reap_ui_child();
        core.process_ipc();
        core.process_tray();
        core.process_mpris();
        core.process_shortcuts();
        core.process_ui_cmds();
        core.process_picked_files();

        core.audio.maintain();
        core.tick();
        core.sync_outputs();

        let ui_connected = core
            .bridge
            .as_ref()
            .map(|b| b.has_client())
            .unwrap_or(false);
        if ui_connected {
            core.ui_was_connected = true;
            core.push_snapshot_if_needed();
            if core.playing || core.timer_end.is_some() || core.notice_until.is_some() {
                thread::park_timeout(Duration::from_millis(160));
            } else {
                // Zero busy-polling even while fallback UI is visible and paused
                thread::park();
            }
        } else {
            if core.ui_was_connected {
                core.ui_was_connected = false;
                core.last_pushed_snap = None;
                if let Some(mut child) = core.ui_child.take() {
                    thread::spawn(move || {
                        let _ = child.wait();
                    });
                }
                if core.tray.is_none() && !core.gnome_popover_active {
                    core.running = false;
                    break;
                }
            }
            if core.playing || core.timer_end.is_some() || core.notice_until.is_some() {
                thread::park_timeout(Duration::from_millis(500));
            } else {
                // True 0% idle CPU when paused
                thread::park();
            }
        }
    }

    core.close_ui();
    core.save_settings();
}

struct Core {
    main_thread: Arc<thread::Thread>,
    audio: AudioEngine,
    tray: Option<Tray>,
    ipc: Option<IpcServer>,
    bridge: Option<BridgeServer>,
    mpris: Option<MprisServer>,
    mpris_rx: Receiver<MprisCmd>,
    file_pick_tx: Sender<(PathBuf, bool)>,
    file_pick_rx: Receiver<(PathBuf, bool)>,
    picking_file: Arc<AtomicBool>,
    shortcuts: ShortcutManager,
    ui_child: Option<Child>,
    ui_was_connected: bool,
    running: bool,

    is_file_mode: bool,
    active_sound_idx: usize,
    active_file: Option<PathBuf>,
    locked_default_file: Option<PathBuf>,
    local_tracks: Vec<LocalTrack>,
    file_tone: FileTone,
    loop_crossfade: bool,
    layer_sound: Option<Sound>,
    layer_mix: u8,

    playing: bool,
    has_played: bool,
    elapsed: Duration,
    last_tick: Instant,
    timer_minutes: u32,
    timer_remaining: Option<Duration>,
    timer_end: Option<Instant>,

    theme_mode: ThemeMode,
    reduce_motion: bool,
    window_mode: WindowMode,
    dock_anchor: DockAnchor,
    saved_pos: Option<[f32; 2]>,
    pinned: bool,
    auto_play_on_launch: bool,
    start_in_background: bool,
    gnome_pill_opted_out: bool,
    autostart_enabled: bool,
    gnome_pill_installed: bool,
    gnome_popover_active: bool,
    global_shortcut_toggle: String,
    global_shortcut_play: String,

    rms_history: VecDeque<f32>,
    bar_heights: [f32; BAR_COUNT],
    notice: Option<(String, bool)>,
    notice_until: Option<Instant>,
    missing_locked_file: Option<PathBuf>,
    last_pushed_snap: Option<UiSnapshot>,
    last_rms_push: Instant,
}

impl Core {
    fn current_sound(&self) -> Sound {
        Sound::ALL[self.active_sound_idx]
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
        if self.is_file_mode {
            if self.locked_default_file.is_some() && self.locked_default_file == self.active_file {
                "locked default".to_string()
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

    fn set_notice(&mut self, msg: impl Into<String>, is_err: bool) {
        self.notice = Some((msg.into(), is_err));
        self.notice_until = Some(Instant::now() + Duration::from_secs(if is_err { 6 } else { 3 }));
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
            gnome_pill_opted_out: self.gnome_pill_opted_out,
            global_shortcut_toggle: self.global_shortcut_toggle.clone(),
            global_shortcut_play: self.global_shortcut_play.clone(),
        };
        settings::save(&s);
    }

    fn reap_ui_child(&mut self) {
        if let Some(child) = &mut self.ui_child {
            if let Ok(Some(_)) = child.try_wait() {
                self.ui_child = None;
            }
        }
    }

    fn ui_is_open(&mut self) -> bool {
        self.reap_ui_child();
        self.bridge
            .as_ref()
            .map(|b| b.has_client())
            .unwrap_or(false)
            || self.ui_child.is_some()
            || crate::ipc::InstanceLock::is_locked("zenhush-ui")
    }

    fn open_ui(&mut self, activation_token: Option<String>) {
        if self.ui_is_open() {
            if let Some(ref b) = self.bridge {
                b.send(&CoreToUi::Focus);
            }
            return;
        }
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let mut cmd = Command::new(exe);
        cmd.arg("--ui");
        if let Some(tok) = activation_token {
            cmd.env("XDG_ACTIVATION_TOKEN", tok);
        }
        if let Ok(child) = cmd.spawn() {
            self.ui_child = Some(child);
        }
    }

    fn close_ui(&mut self) {
        if let Some(ref b) = self.bridge {
            b.send(&CoreToUi::Close);
        }
    }

    fn toggle_or_open_primary(&mut self, activation_token: Option<String>) {
        if !self.gnome_popover_active && gnome_ext::is_active() {
            self.gnome_popover_active = true;
        }
        if self.gnome_popover_active {
            if let Some(ref mpris) = self.mpris {
                mpris.emit_toggle_popover();
                return;
            }
        }
        self.toggle_ui(activation_token);
    }

    fn toggle_ui(&mut self, activation_token: Option<String>) {
        if self.ui_is_open() {
            self.close_ui();
        } else {
            self.open_ui(activation_token);
        }
    }

    fn spawn_file_picker(&self, lock_default: bool) {
        if self.picking_file.swap(true, Ordering::AcqRel) {
            return;
        }
        let tx = self.file_pick_tx.clone();
        let wake = self.main_thread.clone();
        let flag = self.picking_file.clone();
        thread::spawn(move || {
            let picked = rfd::FileDialog::new()
                .set_title("Choose local focus audio")
                .add_filter("Audio", &["wav", "mp3", "flac", "ogg", "m4a"])
                .pick_file();
            flag.store(false, Ordering::Release);
            if let Some(path) = picked {
                let _ = tx.send((path, lock_default));
            }
            wake.unpark();
        });
    }

    fn process_picked_files(&mut self) {
        let mut items = Vec::new();
        while let Ok(item) = self.file_pick_rx.try_recv() {
            items.push(item);
        }
        for (p, lock_default) in items {
            self.add_local_file(p, true, lock_default);
        }
    }

    fn play_selected(&mut self) {
        self.has_played = true;
        if self.is_file_mode {
            let Some(path) = self.active_file.clone() else {
                self.spawn_file_picker(false);
                return;
            };
            let handle = FileStreamHandle::new(path, self.loop_crossfade);
            let name = self.active_track_title();
            match self.audio.play_file(handle, name) {
                Ok(()) => {
                    self.playing = true;
                    self.missing_locked_file = None;
                    if matches!(self.notice, Some((_, true))) {
                        self.notice = None;
                    }
                }
                Err(e) => {
                    self.playing = false;
                    self.set_notice(e, true);
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
    }

    fn toggle_play(&mut self) {
        if self.playing {
            self.audio.pause();
            self.playing = false;
            self.freeze_timer();
        } else if self.has_played
            && self.audio.has_loaded_source()
            && self.audio.is_file() == self.is_file_mode
        {
            self.audio.resume();
            self.playing = true;
            self.resume_timer();
        } else {
            self.play_selected();
        }
    }

    fn restart_playback(&mut self) {
        self.has_played = true;
        if self.is_file_mode && self.active_file.is_none() {
            self.spawn_file_picker(false);
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
    }

    fn select_live_sound(&mut self, sound: Sound) {
        if let Some(idx) = Sound::ALL.iter().position(|&s| s == sound) {
            self.active_sound_idx = idx;
            self.is_file_mode = false;
            self.save_settings();
            if self.playing {
                self.play_selected();
            }
        }
    }

    fn select_file_mode(&mut self) {
        if self.active_file.is_none() {
            if let Some(first) = self.local_tracks.first().map(|t| t.path.clone()) {
                self.active_file = Some(first);
            } else {
                self.spawn_file_picker(false);
                return;
            }
        }
        self.is_file_mode = true;
        self.save_settings();
        if self.playing {
            self.play_selected();
        }
    }

    fn remove_active_file(&mut self) {
        if let Some(p) = self.active_file.take() {
            self.local_tracks.retain(|t| t.path != p);
            if self.locked_default_file.as_ref() == Some(&p) {
                self.locked_default_file = None;
            }
        }
        self.active_file = self.local_tracks.first().map(|t| t.path.clone());
        if self.active_file.is_none() || self.is_file_mode {
            self.is_file_mode = false;
            if self.playing {
                self.play_selected();
            }
        }
        self.save_settings();
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
        }
    }

    fn add_local_file(&mut self, path: PathBuf, play_now: bool, lock_default: bool) {
        match filemode::probe_file(&path) {
            Ok(probe) => {
                let title = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Audio Track".to_string());
                if let Some(existing) = self.local_tracks.iter_mut().find(|t| t.path == path) {
                    existing.title = title;
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
                    self.locked_default_file = Some(path);
                    self.set_notice("Locked as default audio", false);
                }
                self.save_settings();
                if play_now {
                    self.play_selected();
                }
            }
            Err(e) => self.set_notice(e, true),
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
    }

    fn cycle_timer(&mut self) {
        let idx = TIMER_CYCLE
            .iter()
            .position(|&m| m == self.timer_minutes)
            .unwrap_or(0);
        self.timer_minutes = TIMER_CYCLE[(idx + 1) % TIMER_CYCLE.len()];
        self.arm_timer();
        self.save_settings();
    }

    fn arm_timer(&mut self) {
        if self.timer_minutes == 0 {
            self.timer_remaining = None;
            self.timer_end = None;
            self.audio.set_master_fade(1.0);
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
                } else {
                    let rem = end.duration_since(now);
                    self.timer_remaining = Some(rem);
                    let secs_left = rem.as_secs_f32();
                    if secs_left <= 3.0 {
                        self.audio
                            .set_master_fade((secs_left / 3.0).clamp(0.0, 1.0));
                    }
                }
            }
        }

        for v in self.audio.drain_rms() {
            if self.rms_history.len() == 32 {
                self.rms_history.pop_front();
            }
            self.rms_history.push_back(v);
        }

        if self.playing && !self.reduce_motion && !self.rms_history.is_empty() && self.ui_is_open()
        {
            let len = self.rms_history.len();
            for i in 0..BAR_COUNT {
                let idx = len.saturating_sub(BAR_COUNT - i);
                let target = self.rms_history.get(idx).copied().unwrap_or(0.14);
                self.bar_heights[i] += (target - self.bar_heights[i]) * 0.42;
            }
        }
    }

    fn process_ipc(&mut self) {
        let cmds: Vec<RemoteCmd> = match &self.ipc {
            Some(s) => {
                let mut v = Vec::new();
                while let Ok(c) = s.rx.try_recv() {
                    v.push(c);
                }
                v
            }
            None => Vec::new(),
        };
        for cmd in cmds {
            match cmd {
                RemoteCmd::Toggle => self.toggle_or_open_primary(None),
                RemoteCmd::Show => self.toggle_or_open_primary(None),
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
                RemoteCmd::VolUp => {
                    self.audio.set_volume(self.audio.volume().saturating_add(5));
                    self.save_settings();
                }
                RemoteCmd::VolDown => {
                    self.audio.set_volume(self.audio.volume().saturating_sub(5));
                    self.save_settings();
                }
                RemoteCmd::Mute => {
                    self.audio.set_muted(!self.audio.is_muted());
                    self.save_settings();
                }
                RemoteCmd::ToggleNotch => {
                    self.window_mode = match self.window_mode {
                        WindowMode::Card => WindowMode::NotchPill,
                        WindowMode::NotchPill => WindowMode::Card,
                    };
                    self.save_settings();
                    if !self.gnome_popover_active && !self.ui_is_open() {
                        self.open_ui(None);
                    }
                }
                RemoteCmd::Quit => self.running = false,
            }
        }
    }

    fn process_tray(&mut self) {
        let evs: Vec<TrayEvent> = match &self.tray {
            Some(t) => {
                let mut v = Vec::new();
                while let Ok(e) = t.rx.try_recv() {
                    v.push(e);
                }
                v
            }
            None => Vec::new(),
        };
        for ev in evs {
            match ev {
                TrayEvent::ToggleWindow => {
                    #[cfg(target_os = "linux")]
                    let tok = ksni::current_xdg_activation_token();
                    #[cfg(not(target_os = "linux"))]
                    let tok = None;
                    self.toggle_or_open_primary(tok);
                }
                TrayEvent::TogglePlay => self.toggle_play(),
                TrayEvent::NextSound => self.step_sound(1),
                TrayEvent::PrevSound => self.step_sound(-1),
                TrayEvent::SelectSound(s) => self.select_live_sound(s),
                TrayEvent::SelectFile => self.select_file_mode(),
                TrayEvent::PickFile => self.spawn_file_picker(false),
                TrayEvent::ToggleLockDefault => {
                    if let Some(p) = self.active_file.clone() {
                        self.toggle_lock_for_path(&p);
                    }
                }
                TrayEvent::SetVolume(v) => {
                    self.audio.set_volume(v);
                    self.save_settings();
                }
                TrayEvent::ToggleMute => {
                    self.audio.set_muted(!self.audio.is_muted());
                    self.save_settings();
                }
                TrayEvent::SetTimer(m) => {
                    self.timer_minutes = m;
                    self.arm_timer();
                    self.save_settings();
                }
                TrayEvent::Quit => self.running = false,
            }
        }
    }

    fn process_mpris(&mut self) {
        let cmds: Vec<MprisCmd> = self.mpris_rx.try_iter().collect();
        for cmd in cmds {
            match cmd {
                MprisCmd::Raise => self.toggle_or_open_primary(None),
                MprisCmd::Quit => self.running = false,
                MprisCmd::Play => {
                    if !self.playing {
                        self.toggle_play();
                    }
                }
                MprisCmd::Pause | MprisCmd::Stop => {
                    if self.playing {
                        self.toggle_play();
                    }
                }
                MprisCmd::PlayPause => self.toggle_play(),
                MprisCmd::Next => self.step_sound(1),
                MprisCmd::Previous => self.step_sound(-1),
                MprisCmd::SetVolume(v) => {
                    self.audio.set_volume(v);
                    self.save_settings();
                }
                MprisCmd::SelectSound(idx) => {
                    if let Some(&s) = Sound::ALL.get(idx as usize) {
                        self.select_live_sound(s);
                    }
                }
                MprisCmd::SelectFile => self.select_file_mode(),
                MprisCmd::PickFile => self.spawn_file_picker(false),
                MprisCmd::PickAndLockFile => self.spawn_file_picker(true),
                MprisCmd::ToggleLockDefault => {
                    if let Some(p) = self.active_file.clone() {
                        self.toggle_lock_for_path(&p);
                    }
                }
                MprisCmd::RemoveFile => self.remove_active_file(),
                MprisCmd::ToggleMute => {
                    self.audio.set_muted(!self.audio.is_muted());
                    self.save_settings();
                }
                MprisCmd::CycleTimer => self.cycle_timer(),
                MprisCmd::ToggleAutostart => {
                    let target = !self.autostart_enabled;
                    if shortcuts::set_autostart(target).is_ok() {
                        self.autostart_enabled = target;
                    }
                }
                MprisCmd::OpenWindow => self.open_ui(None),
            }
        }
    }

    fn process_shortcuts(&mut self) {
        for act in self.shortcuts.poll() {
            match act {
                GlobalAction::TogglePopup => self.toggle_or_open_primary(None),
                GlobalAction::TogglePlay => self.toggle_play(),
            }
        }
    }

    fn process_ui_cmds(&mut self) {
        let cmds: Vec<UiCmd> = match &self.bridge {
            Some(b) => {
                let mut v = Vec::new();
                while let Ok(c) = b.cmd_rx.try_recv() {
                    v.push(c);
                }
                v
            }
            None => Vec::new(),
        };
        for cmd in cmds {
            match cmd {
                UiCmd::TogglePlay => self.toggle_play(),
                UiCmd::Restart => self.restart_playback(),
                UiCmd::StepSound(d) => self.step_sound(d),
                UiCmd::SelectSound(s) => self.select_live_sound(s),
                UiCmd::SelectFileMode => self.select_file_mode(),
                UiCmd::PickFile { lock_default } => self.spawn_file_picker(lock_default),
                UiCmd::AddLocalFile {
                    path,
                    play_now,
                    lock_default,
                } => self.add_local_file(path, play_now, lock_default),
                UiCmd::RemoveLocalTrack(p) => {
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
                }
                UiCmd::ToggleLockDefault(p) => self.toggle_lock_for_path(&p),
                UiCmd::UnlockMissingDefault => {
                    self.locked_default_file = None;
                    self.missing_locked_file = None;
                    self.is_file_mode = false;
                    self.notice = None;
                    self.save_settings();
                }
                UiCmd::SetFileTone(t) => {
                    self.file_tone = t;
                    self.audio.set_file_tone(t);
                    self.save_settings();
                }
                UiCmd::SetLoopCrossfade(x) => {
                    self.loop_crossfade = x;
                    self.audio.set_crossfade(x);
                    self.save_settings();
                }
                UiCmd::SetLayer(l) => {
                    self.layer_sound = l;
                    self.audio.set_layer(l);
                    self.save_settings();
                }
                UiCmd::SetLayerMix(m) => {
                    self.layer_mix = m;
                    self.audio.set_layer_mix(m);
                    self.save_settings();
                }
                UiCmd::SetVolume(v) => {
                    self.audio.set_volume(v);
                    self.save_settings();
                }
                UiCmd::ToggleMute => {
                    self.audio.set_muted(!self.audio.is_muted());
                    self.save_settings();
                }
                UiCmd::SetDepth(d) => {
                    self.audio.set_depth(d);
                    self.save_settings();
                }
                UiCmd::SetRumble(r) => {
                    self.audio.set_rumble(r);
                    self.save_settings();
                }
                UiCmd::ResetControls => {
                    self.audio.set_volume(DEFAULT_VOLUME);
                    self.audio.set_muted(false);
                    self.audio.set_depth(DEFAULT_DEPTH);
                    self.audio.set_rumble(DEFAULT_RUMBLE);
                    self.layer_sound = None;
                    self.audio.set_layer(None);
                    self.save_settings();
                }
                UiCmd::SetTimer(m) => {
                    self.timer_minutes = m;
                    self.arm_timer();
                    self.save_settings();
                }
                UiCmd::CycleTimer => self.cycle_timer(),
                UiCmd::SetTheme(t) => {
                    self.theme_mode = t;
                    self.save_settings();
                }
                UiCmd::SetReduceMotion(rm) => {
                    self.reduce_motion = rm;
                    self.save_settings();
                }
                UiCmd::SetWindowMode(wm) => {
                    self.window_mode = wm;
                    self.save_settings();
                }
                UiCmd::SetDockAnchor(a) => {
                    self.dock_anchor = a;
                    self.save_settings();
                }
                UiCmd::SetSavedPos(pos) => {
                    self.saved_pos = pos;
                    self.save_settings();
                }
                UiCmd::SetPinned(p) => {
                    self.pinned = p;
                    self.save_settings();
                }
                UiCmd::SetAutoPlay(a) => {
                    self.auto_play_on_launch = a;
                    self.save_settings();
                }
                UiCmd::SetStartInBackground(b) => {
                    self.start_in_background = b;
                    self.save_settings();
                }
                UiCmd::SetAutostart(target) => match shortcuts::set_autostart(target) {
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
                },
                UiCmd::InstallLauncher => match shortcuts::install_to_user_local() {
                    Ok(_) => self.set_notice("Installed to ~/.local/bin & App Launcher", false),
                    Err(e) => self.set_notice(e, true),
                },
                UiCmd::ToggleGnomePill => {
                    let installing = !gnome_ext::is_installed();
                    let res = if installing {
                        gnome_ext::install_gnome_pill()
                    } else {
                        gnome_ext::uninstall_gnome_pill()
                    };
                    match res {
                        Ok(msg) => {
                            self.gnome_pill_opted_out = !installing;
                            self.gnome_pill_installed = gnome_ext::is_installed();
                            self.gnome_popover_active = gnome_ext::is_active();
                            self.save_settings();
                            self.set_notice(msg, false);
                        }
                        Err(e) => self.set_notice(e, true),
                    }
                }
                UiCmd::BindGnomeShortcut => match shortcuts::register_gnome_shortcuts(
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
                },
                UiCmd::SetGlobalShortcutToggle(s) => {
                    self.global_shortcut_toggle = s;
                    self.shortcuts
                        .apply(&self.global_shortcut_toggle, &self.global_shortcut_play);
                    self.save_settings();
                }
                UiCmd::QuitDaemon => self.running = false,
            }
        }
    }

    fn sync_outputs(&self) {
        let timer_label = self
            .timer_remaining
            .filter(|r| r.as_secs() > 0)
            .map(|r| format_duration(r.as_secs()))
            .unwrap_or_default();

        if let Some(ref tray) = self.tray {
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
                visible: self
                    .bridge
                    .as_ref()
                    .map(|b| b.has_client())
                    .unwrap_or(false),
                tooltip,
            });
        }

        if let Some(ref mpris) = self.mpris {
            mpris.sync(&MprisState {
                playing: self.playing,
                has_played: self.has_played,
                is_file: self.is_file_mode,
                sound_idx: self.active_sound_idx as u32,
                title: self.display_title(),
                subtitle: self.display_subtitle(),
                volume: self.audio.volume(),
                muted: self.audio.is_muted(),
                timer_minutes: self.timer_minutes,
                timer_remaining_secs: self.timer_remaining.map(|d| d.as_secs()),
                has_file: self.active_file.is_some(),
                file_title: self.active_track_title(),
                locked_default: self.locked_default_file.is_some()
                    && self.locked_default_file == self.active_file,
                autostart: self.autostart_enabled,
                audio_error: self.audio.audio_error(),
            });
        }
    }

    fn build_snapshot(&self) -> UiSnapshot {
        UiSnapshot {
            playing: self.playing,
            has_played: self.has_played,
            is_file_mode: self.is_file_mode,
            active_sound: self.current_sound(),
            active_file: self.active_file.clone(),
            locked_default_file: self.locked_default_file.clone(),
            local_tracks: self.local_tracks.clone(),
            file_tone: self.file_tone,
            loop_crossfade: self.loop_crossfade,
            layer_sound: self.layer_sound,
            layer_mix: self.layer_mix,
            volume: self.audio.volume(),
            muted: self.audio.is_muted(),
            depth: self.audio.depth(),
            rumble: self.audio.rumble(),
            elapsed_secs: self.elapsed.as_secs(),
            timer_minutes: self.timer_minutes,
            timer_remaining_secs: self.timer_remaining.map(|d| d.as_secs()),
            rms_bars: self.bar_heights.to_vec(),
            audio_error: self.audio.audio_error(),
            device_name: self.audio.device_name().to_string(),
            notice: self.notice.clone(),
            missing_locked_file: self.missing_locked_file.clone(),
            theme_mode: self.theme_mode,
            reduce_motion: self.reduce_motion,
            window_mode: self.window_mode,
            dock_anchor: self.dock_anchor,
            saved_pos: self.saved_pos,
            pinned: self.pinned,
            auto_play_on_launch: self.auto_play_on_launch,
            start_in_background: self.start_in_background,
            autostart_enabled: self.autostart_enabled,
            gnome_pill_installed: self.gnome_pill_installed,
            gnome_popover_active: self.gnome_popover_active,
            picking_file: self.picking_file.load(Ordering::Acquire),
            global_shortcut_toggle: self.global_shortcut_toggle.clone(),
            global_shortcut_play: self.global_shortcut_play.clone(),
            shortcut_status: self.shortcuts.status_note.clone(),
            has_tray: self.tray.is_some(),
        }
    }

    fn push_snapshot_if_needed(&mut self) {
        let Some(ref b) = self.bridge else {
            return;
        };
        let next = self.build_snapshot();
        let should_push = match &self.last_pushed_snap {
            None => true,
            Some(prev) => {
                // Compare everything except rms_bars cleanly; throttle rms-only updates
                let mut prev_no_rms = prev.clone();
                prev_no_rms.rms_bars.clear();
                let mut next_no_rms = next.clone();
                next_no_rms.rms_bars.clear();
                // Ignore pure elapsed_secs increments if paused
                if !next.playing {
                    prev_no_rms.elapsed_secs = next_no_rms.elapsed_secs;
                }
                if prev_no_rms != next_no_rms {
                    true
                } else if self.playing
                    && !self.reduce_motion
                    && self.last_rms_push.elapsed() >= Duration::from_millis(160)
                {
                    let max_delta = prev
                        .rms_bars
                        .iter()
                        .zip(&next.rms_bars)
                        .map(|(a, b)| (a - b).abs())
                        .fold(0.0f32, f32::max);
                    max_delta > 0.02
                } else {
                    false
                }
            }
        };
        if should_push {
            self.last_rms_push = Instant::now();
            self.last_pushed_snap = Some(next.clone());
            b.send(&CoreToUi::Snapshot(Box::new(next)));
        }
    }
}
