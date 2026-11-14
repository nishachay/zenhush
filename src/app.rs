//! The popup window: title, visualizer, transport, sounds, sliders,
//! sleep timer, and settings. Ported from the Python app's Popup with
//! the same behaviour: subtitle follows the sound, the file entry is
//! skipped when no file is chosen, the sleep timer freezes while paused,
//! and a dead audio device shows its error instead of "Live".

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Color32, RichText, Rounding, Stroke, Vec2};

use crate::audio::AudioEngine;
use crate::icons::{IconCache, IconKind};
use crate::filemode;
use crate::settings::{self, Settings, ThemeMode};
use crate::synth::{Sound, DEFAULT_DEPTH, DEFAULT_RUMBLE, DEFAULT_VOLUME};
use crate::theme::{DARK, LIGHT};
use crate::tray::Tray;

/// Timer choices, minutes (0 = off). Matches the Python TIMER_CHOICES.
const TIMER_CHOICES: [(u32, &str); 5] = [
    (0, "Off"),
    (15, "15 min"),
    (30, "30 min"),
    (45, "45 min"),
    (60, "60 min"),
];

/// Sounds in order, plus the file entry at the end.
const SOUND_COUNT: usize = 6;

/// Idle bar heights (Python EQ.IDLE_HEIGHTS). The visualizer shows
/// these when nothing has played yet, and freezes them on pause.
const IDLE_HEIGHTS: [f32; 12] = [
    0.35, 0.55, 0.8, 0.6, 0.45, 0.9, 1.0, 0.5, 0.7, 0.4, 0.55, 0.3,
];

/// Bold (700) and semibold (600) faces, instantiated from the variable
/// font. egui has no weight support, so `.strong()` only recolours;
/// real bold needs an explicit named family (see main.rs).
fn bold_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("SchibstedGroteskBold".into()))
}
fn semibold_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("SchibstedGroteskSemiBold".into()))
}

pub struct ZenhushApp {
    audio: AudioEngine,
    settings: Settings,
    /// Index into sounds: 0-4 live recipes, 5 = your file.
    selected: usize,
    file_path: Option<PathBuf>,
    playing: bool,
    /// Elapsed play time (accumulates only while playing).
    elapsed: Duration,
    last_tick: Instant,
    /// Sleep timer: chosen minutes, and the remaining time (frozen on pause).
    timer_minutes: u32,
    timer_remaining: Option<Duration>,
    timer_end: Option<Instant>,
    theme_mode: ThemeMode,
    reduce_motion: bool,
    settings_open: bool,
    /// Recent RMS values for the visualizer (oldest first).
    rms_history: VecDeque<f32>,
    /// Current bar heights (smoothed toward live RMS while playing,
    /// frozen on pause, idle heights before the first play).
    bar_heights: [f32; 12],
    /// Index of the loudest bar (drawn in the accent color).
    bar_accent: usize,
    /// The system tray (None when no tray is available).
    tray: Option<Tray>,
    /// Last decode/file error to show in the status line.
    notice: Option<String>,
    /// True once the user has pressed play (or stepped a sound).
    /// The Python app leaves the EQ status empty until then.
    has_played: bool,
    /// Rasterized SVG icons (same Lucide files as the Python app).
    icons: IconCache,
    /// Whether the window is currently shown (tray toggles it).
    visible: bool,
    /// True once the window has gained focus at least once. The popup
    /// hides on focus loss, but not before it was ever focused.
    was_focused: bool,
}

impl ZenhushApp {
    pub fn new(audio: AudioEngine, tray: Option<Tray>) -> Self {
        let settings = settings::load();
        let mut app = ZenhushApp {
            audio,
            settings: settings.clone(),
            selected: 0,
            file_path: None,
            playing: false,
            elapsed: Duration::ZERO,
            last_tick: Instant::now(),
            timer_minutes: settings.timer_minutes,
            timer_remaining: None,
            timer_end: None,
            theme_mode: settings.theme,
            reduce_motion: settings.reduce_motion,
            settings_open: false,
            rms_history: VecDeque::with_capacity(64),
            bar_heights: IDLE_HEIGHTS,
            bar_accent: 6,
            has_played: false,
            icons: IconCache::new(),
            tray,
            notice: None,
            visible: true,
            was_focused: false,
        };
        // Apply the remembered sliders to the audio engine.
        app.audio.set_volume(settings.volume);
        app.audio.set_depth(settings.depth);
        app.audio.set_rumble(settings.rumble);
        app
    }

    /// Window height, matching the Python app's _set_fixed_height:
    /// BASE_HEIGHT 296, + SETTINGS_HEIGHT 336 when the settings are
    /// open, + 28 for the Quit row when there is no tray. The Quit
    /// button only shows without a tray, so a tray-less window is
    /// 28px taller in both states.
    fn window_height(&self) -> f32 {
        let mut h = if self.settings_open { 632.0 } else { 296.0 };
        if self.tray.is_none() {
            h += 28.0;
        }
        h
    }

    /// Reset Volume/Depth/Rumble to the defaults (70/65/58),
    /// like the Python app's Reset button.
    fn reset_sound_controls(&mut self) {
        self.audio.set_volume(DEFAULT_VOLUME);
        self.audio.set_depth(DEFAULT_DEPTH);
        self.audio.set_rumble(DEFAULT_RUMBLE);
        self.save_settings();
    }

    fn save_settings(&self) {
        let s = Settings {
            volume: self.audio.volume(),
            depth: self.audio.depth(),
            rumble: self.audio.rumble(),
            theme: self.theme_mode,
            reduce_motion: self.reduce_motion,
            timer_minutes: self.timer_minutes,
        };
        settings::save(&s);
    }

    fn theme(&self, ctx: &egui::Context) -> crate::theme::Theme {
        match self.theme_mode {
            ThemeMode::Light => LIGHT,
            ThemeMode::Dark => DARK,
            ThemeMode::Auto => {
                // Follow the OS. Match Qt: dark only when the system
                // says dark; light when it says light OR unknown.
                // (In a bare X session the scheme is unknown, and the
                // Python app shows light there.)
                if ctx.system_theme() == Some(egui::Theme::Dark) {
                    DARK
                } else {
                    LIGHT
                }
            }
        }
    }

    fn sound_title(&self) -> String {
        if self.selected == 5 {
            "Your File".to_string()
        } else {
            Sound::ALL[self.selected].title().to_string()
        }
    }

    fn sound_subtitle(&self) -> String {
        // Idle: the Python app shows "press play to focus" until the
        // user presses play. Playing: the sound's subtitle (or the
        // file name for a looped file).
        if !self.playing {
            return "press play to focus".to_string();
        }
        if self.selected == 5 {
            if let Some(p) = &self.file_path {
                let base = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "file".to_string());
                if base.chars().count() <= 30 {
                    base
                } else {
                    let mut s: String = base.chars().take(27).collect();
                    s.push('…');
                    s
                }
            } else {
                "loop a local audio file".to_string()
            }
        } else {
            Sound::ALL[self.selected].subtitle().to_string()
        }
    }

    /// Status text: Live / File / Paused / countdown / audio error.
    /// Empty until the user first presses play (the Python app leaves
    /// the EQ status blank at startup; _render_status only runs on a
    /// state change). The audio error is short on purpose: the full
    /// device error is long and would overflow the visualizer.
    fn status_text(&self) -> String {
        if !self.has_played {
            return String::new();
        }
        if let Some(err) = self.audio.audio_error() {
            // A dead device beats everything: say so, don't show "Live".
            return "No audio device".to_string();
        }
        if let Some(rem) = self.timer_remaining {
            // The countdown is showing only while the timer is armed.
            if self.playing || rem.as_millis() > 0 {
                let secs = rem.as_secs();
                return format!("{}:{:02}", secs / 60, secs % 60);
            }
        }
        if !self.playing {
            return "Paused".to_string();
        }
        if self.audio.is_file() {
            "File".to_string()
        } else {
            "Live".to_string()
        }
    }

    /// Start playing the currently selected sound.
    fn play_selected(&mut self) {
        self.has_played = true;
        if self.selected == 5 {
            if self.file_path.is_none() {
                // No file yet: ask for one; if cancelled, stay put.
                if !self.choose_file() {
                    return;
                }
            }
            if let Some(path) = &self.file_path {
                match filemode::decode(path) {
                    Ok(samples) => {
                        let name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "file".to_string());
                        self.audio
                            .play_file(std::sync::Arc::new(samples), name);
                        self.playing = true;
                        self.notice = None;
                    }
                    Err(e) => {
                        self.notice = Some(e);
                        return;
                    }
                }
            }
        } else {
            self.audio.play_sound(Sound::ALL[self.selected]);
            self.playing = true;
            self.notice = None;
        }
        self.elapsed = Duration::ZERO;
        self.resume_timer();
    }

    /// Ask the user to pick an audio file. Returns true when one was chosen.
    fn choose_file(&mut self) -> bool {
        let picked = rfd::FileDialog::new()
            .add_filter("Audio", &["wav", "mp3", "flac", "ogg", "m4a"])
            .pick_file();
        match picked {
            Some(p) => {
                self.file_path = Some(p);
                true
            }
            None => false,
        }
    }

    fn toggle_play(&mut self) {
        if self.playing {
            self.audio.pause();
            self.playing = false;
            self.freeze_timer();
        } else {
            // Resume: if nothing ever played, start the selected sound.
            if self.audio.is_playing() {
                self.audio.resume();
            } else {
                self.play_selected();
                return;
            }
            self.playing = true;
            self.resume_timer();
        }
    }

    /// Step to the previous/next sound. Skips the file entry when no
    /// file is chosen (so title and audio can never disagree).
    fn step_sound(&mut self, dir: i32) {
        let mut next = self.selected as i32 + dir;
        if next < 0 {
            next = (SOUND_COUNT - 1) as i32;
        }
        if next >= SOUND_COUNT as i32 {
            next = 0;
        }
        let mut next = next as usize;
        // Skip the file entry when no file is chosen.
        if next == 5 && self.file_path.is_none() {
            next = if dir > 0 { 0 } else { 4 };
        }
        self.selected = next;
        if self.playing {
            self.play_selected();
        }
    }

    fn restart(&mut self) {
        self.audio.restart();
        self.elapsed = Duration::ZERO;
    }

    /// Arm (or re-arm) the sleep timer for the chosen minutes.
    fn arm_timer(&mut self) {
        if self.timer_minutes == 0 {
            self.timer_remaining = None;
            self.timer_end = None;
            return;
        }
        let dur = Duration::from_secs(self.timer_minutes as u64 * 60);
        self.timer_remaining = Some(dur);
        if self.playing {
            self.timer_end = Some(Instant::now() + dur);
        } else {
            self.timer_end = None;
        }
    }

    /// Freeze the countdown while paused (the timer must not run
    /// while the sound is stopped).
    fn freeze_timer(&mut self) {
        if let Some(end) = self.timer_end {
            let now = Instant::now();
            let remaining = end.saturating_duration_since(now);
            self.timer_remaining = Some(remaining);
            self.timer_end = None;
        }
    }

    /// Resume the countdown from the frozen remaining time.
    fn resume_timer(&mut self) {
        if let Some(rem) = self.timer_remaining {
            if rem.as_millis() > 0 {
                self.timer_end = Some(Instant::now() + rem);
            }
        }
    }

    /// Called every frame: advance elapsed time and the countdown.
    fn tick(&mut self) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick);
        self.last_tick = now;
        if self.playing {
            self.elapsed += dt;
            if let Some(end) = self.timer_end {
                if now >= end {
                    // Timer expired: pause and reset the timer.
                    self.timer_remaining = None;
                    self.timer_end = None;
                    self.timer_minutes = 0;
                    self.audio.pause();
                    self.playing = false;
                    self.save_settings();
                } else {
                    self.timer_remaining = Some(end.duration_since(now));
                }
            }
        }
        // Drain new RMS values into the visualizer history.
        for v in self.audio.drain_rms() {
            if self.rms_history.len() == 64 {
                self.rms_history.pop_front();
            }
            self.rms_history.push_back(v);
        }
        // Animate the bars toward the live RMS (Python EQ._tick).
        // Paused: frozen. Reduce motion: frozen too.
        if self.playing && !self.reduce_motion && !self.rms_history.is_empty() {
            let n = 12;
            let len = self.rms_history.len();
            let mut loudest = 0;
            let mut loudest_v = -1.0f32;
            for i in 0..n {
                let idx = len.saturating_sub(n - i);
                let target = self.rms_history.get(idx).copied().unwrap_or(0.15);
                self.bar_heights[i] += (target - self.bar_heights[i]) * 0.45;
                if self.bar_heights[i] > loudest_v {
                    loudest_v = self.bar_heights[i];
                    loudest = i;
                }
            }
            self.bar_accent = loudest;
        }
    }
}

impl eframe::App for ZenhushApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Tray events: left-click toggles, menu Show / Quit.
        // Drain first (so the borrow of the receiver ends before we
        // mutate the app), then act on each event.
        let tray_events: Vec<crate::tray::TrayEvent> = match &self.tray {
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
                crate::tray::TrayEvent::Toggle => {
                    self.visible = !self.visible;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(self.visible));
                    if self.visible {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    }
                }
                crate::tray::TrayEvent::Show => {
                    self.visible = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                crate::tray::TrayEvent::Quit => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        // Hovering the tray icon shows what is playing (or why not).
        if let Some(tray) = &self.tray {
            tray.set_tooltip(&format!(
                "Zenhush — {} ({})",
                self.sound_title(),
                self.status_text()
            ));
        }
        // Hide the popup when it loses focus (tray popups behave this way),
        // but only after it was focused at least once.
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        if focused {
            self.was_focused = true;
        }
        if self.visible && self.was_focused && !focused {
            self.visible = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }

        // Follow the default output device (poll: cpal has no change event).
        self.audio.rebuild_if_device_changed();

        self.tick();

        let theme = self.theme(ctx);
        // Apply the palette to egui's visuals. Match Qt: dark only
        // when the system says dark, else light.
        let mut visuals = egui::Visuals::light();
        if self.theme_mode == ThemeMode::Dark
            || (self.theme_mode == ThemeMode::Auto
                && ctx.system_theme() == Some(egui::Theme::Dark))
        {
            visuals = egui::Visuals::dark();
        }
        visuals.window_fill = theme.card;
        visuals.panel_fill = theme.card;
        visuals.widgets.noninteractive.bg_fill = theme.card;
        visuals.widgets.inactive.bg_fill = theme.field;
        visuals.widgets.hovered.bg_fill = theme.hover;
        visuals.widgets.active.bg_fill = theme.field;
        visuals.selection.bg_fill = theme.accent;
        visuals.selection.stroke = Stroke::new(1.0, theme.accent);
        ctx.set_visuals(visuals);
        // Repaint while playing (visualizer + clock) or while the timer runs.
        if self.playing || self.timer_end.is_some() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }

        // The popup matches the Python app: a compact card (300x296)
        // with the visualizer on top, the title, the transport row,
        // and a Quit text link. Everything else lives in the settings
        // panel behind the gear, which grows the window to 632px.
        let frame = egui::Frame::default()
            .fill(theme.card)
            .corner_radius(24.0)
            .stroke(egui::Stroke::new(1.0, theme.edge))
            .inner_margin(egui::Margin::same(12));
        egui::CentralPanel::default()
            .frame(frame)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);

                // --- Visualizer card: 118px, inset, 12 thick bars ---
                let eq_w = ui.available_width();
                let (eq_rect, _) = ui.allocate_exact_size(
                    Vec2::new(eq_w, 118.0),
                    egui::Sense::hover(),
                );
                let p = ui.painter_at(eq_rect);
                p.rect(
                    eq_rect,
                    Rounding::same(12),
                    theme.inset,
                    Stroke::NONE,
                    egui::StrokeKind::Inside,
                );
                // Bars: margin 14, top 12, bottom 30 (Python EQ).
                let margin = 14.0;
                let top = 12.0;
                let bottom = 30.0;
                let bar_h = eq_rect.height() - margin - bottom - top;
                let n = 12;
                let slot = (eq_rect.width() - 2.0 * margin) / n as f32;
                let bw = slot * 0.55;
                for i in 0..n {
                    let h = (self.bar_heights[i] * bar_h).max(3.0);
                    let x = eq_rect.min.x + margin + i as f32 * slot + (slot - bw) / 2.0;
                    let y = eq_rect.min.y + top + bar_h - h;
                    let color = if i == self.bar_accent {
                        theme.accent
                    } else if self.bar_heights[i] > 0.62 {
                        theme.bar_strong
                    } else {
                        theme.bar
                    };
                    p.rect_filled(
                        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(bw, h)),
                        Rounding::same((bw / 2.0).max(1.0) as u8),
                        color,
                    );
                }
                // Elapsed, bottom-left; status, bottom-right.
                let secs = self.elapsed.as_secs();
                p.text(
                    egui::pos2(eq_rect.min.x + margin, eq_rect.max.y - 10.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("{}:{:02}", secs / 60, secs % 60),
                    egui::FontId::new(12.0, egui::FontFamily::Proportional),
                    theme.subtext,
                );
                let status = self.status_text();
                if !status.is_empty() {
                    p.text(
                        egui::pos2(eq_rect.max.x - margin, eq_rect.max.y - 10.0),
                        egui::Align2::RIGHT_BOTTOM,
                        status,
                        egui::FontId::new(12.0, egui::FontFamily::Proportional),
                        theme.accent_text,
                    );
                }
                if let Some(notice) = &self.notice {
                    ui.label(
                        RichText::new(notice)
                            .size(12.0)
                            .color(theme.accent_text),
                    );
                }
                // Python adds 14px between the EQ card and the title row.
                ui.add_space(14.0);

                // --- Title row: prev, centered title + subtitle, next ---
                ui.horizontal(|ui| {
                    if icon_button(ui, &mut self.icons, IconKind::ChevronLeft, 32.0, 18, theme.subtext, theme, "Previous sound") {
                        self.step_sound(-1);
                    }
                    // The title takes the middle and centers its text
                    // between the two chevrons (Python: stretch, title,
                    // stretch).
                    let title_w = (ui.available_width() - 32.0 - 8.0).max(10.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(title_w, 32.0),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(0.0, 2.0);
                            ui.label(
                                RichText::new(self.sound_title())
                                    .font(bold_font(16.0))
                                    .color(theme.text),
                            );
                            ui.label(
                                RichText::new(self.sound_subtitle())
                                    .size(13.0)
                                    .color(theme.subtext),
                            );
                        },
                    );
                    if icon_button(ui, &mut self.icons, IconKind::ChevronRight, 32.0, 18, theme.subtext, theme, "Next sound") {
                        self.step_sound(1);
                    }
                });
                ui.add_space(14.0);

                // --- Transport: restart, play (64), settings gear ---
                // Spacing 28 between the controls, centered as a group.
                // (horizontal_centered would take all remaining vertical
                // space and push the row down, so center it by hand.)
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(28.0, 0.0);
                    // Row width: 40 + 28 + 64 + 28 + 40 = 200.
                    let row_w = 200.0;
                    ui.add_space(((ui.available_width() - row_w) / 2.0).max(0.0));
                    if icon_button(ui, &mut self.icons, IconKind::Restart, 40.0, 20, theme.subtext, theme, "Restart playback") {
                        self.restart();
                    }
                    // Play/pause: a 64px accent circle with the real
                    // Lucide glyph (Python fills play/pause solid).
                    let (play_rect, play_resp) =
                        ui.allocate_exact_size(Vec2::splat(64.0), egui::Sense::click());
                    let p = ui.painter_at(play_rect);
                    // Pressed: Python fills with `pressed` while down,
                    // so the press reads instantly (Emil: a button
                    // must feel responsive).
                    let fill = if play_resp.is_pointer_button_down_on() {
                        theme.pressed
                    } else {
                        theme.accent
                    };
                    p.circle_filled(play_rect.center(), 31.0, fill);
                    // Focus ring: Python draws a 2px `accent_text`
                    // ring (58px circle) when the button has focus.
                    if play_resp.has_focus() {
                        p.circle_stroke(
                            play_rect.center(),
                            29.0,
                            Stroke::new(2.0, theme.accent_text),
                        );
                    }
                    let glyph = if self.playing {
                        IconKind::Pause
                    } else {
                        IconKind::Play
                    };
                    if let Some(tex) = self.icons.texture(
                        ui.ctx(),
                        glyph,
                        theme.on_accent,
                        26,
                    ) {
                        // Optical alignment (Emil): a right-pointing
                        // triangle's visual weight sits on its base,
                        // so it looks left of centre. Nudge the Play
                        // glyph ~1.5px toward its point; Pause is
                        // symmetric and needs no nudge.
                        let nudge = if self.playing { 0.0 } else { 1.5 };
                        let r = egui::Rect::from_center_size(
                            play_rect.center() + egui::vec2(nudge, 0.0),
                            egui::vec2(26.0, 26.0),
                        );
                        p.image(
                            tex.id(),
                            r,
                            egui::Rect::from_min_max(
                                egui::pos2(0.0, 0.0),
                                egui::pos2(1.0, 1.0),
                            ),
                            Color32::WHITE,
                        );
                    }
                    if play_resp
                        .on_hover_text(if self.playing {
                            "Pause playback"
                        } else {
                            "Play sound"
                        })
                        .clicked()
                    {
                        self.toggle_play();
                    }
                    if icon_button(
                        ui,
                        &mut self.icons,
                        IconKind::Gear,
                        40.0,
                        20,
                        if self.settings_open {
                            theme.accent_text
                        } else {
                            theme.subtext
                        },
                        theme,
                        if self.settings_open {
                            "Close settings"
                        } else {
                            "Settings"
                        },
                    ) {
                        self.settings_open = !self.settings_open;
                        // Grow/shrink the window like the Python popup
                        // (tray-aware: +28 for the Quit row when there
                        // is no tray).
                        let h = self.window_height();
                        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                            300.0, h,
                        )));
                    }
                });

                // --- Quit text link (only when there is no tray) ---
                // The Python app shows it exactly here when no tray is
                // available; with a tray, Quit lives in the tray menu.
                // Python's Quit button stretches to fill the remaining
                // height (Qt gives the slack to the expandable button),
                // so its text sits near the bottom of the card. When
                // the settings are open the settings follow, so the
                // Quit keeps its natural 28px row height there.
                if self.tray.is_none() {
                    let quit_h = if self.settings_open {
                        28.0
                    } else {
                        ui.available_height().max(28.0)
                    };
                    ui.horizontal(|ui| {
                        ui.add_space((ui.available_width() - 90.0).max(0.0) / 2.0);
                        // Python textbtn: transparent background, no frame;
                        // hover only changes the text color.
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Quit Zenhush")
                                        .size(12.0)
                                        .color(theme.accent_text),
                                )
                                .frame(false)
                                .min_size(egui::vec2(90.0, quit_h)),
                            )
                            .clicked()
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                }

                // --- Settings panel (behind the gear) ---
                if self.settings_open {
                    ui.add_space(12.0);
                    ui.spacing_mut().item_spacing = egui::vec2(0.0, 8.0);
                    let pad = 4.0;

                    // SOUND header + Reset.
                    ui.horizontal(|ui| {
                        ui.add_space(pad);
                        ui.label(
                            RichText::new("SOUND")
                                .font(semibold_font(11.0))
                                .color(theme.subtext),
                        );
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                ui.add_space(pad);
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("Reset")
                                                .size(12.0)
                                                .color(theme.accent_text),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                                {
                                    self.reset_sound_controls();
                                }
                            },
                        );
                    });

                    // Sliders: label (52px), slider, value (24px).
                    let vol = self.audio.volume();
                    let dep = self.audio.depth();
                    let rum = self.audio.rumble();
                    let mut slider_row = |ui: &mut egui::Ui,
                                          label: &str,
                                          value: u8,
                                          tip: &str,
                                          set: &mut dyn FnMut(&mut AudioEngine, u8)| {
                        ui.horizontal(|ui| {
                            ui.add_space(pad);
                            // Fixed 52px label column: measure the label and
                            // fill the rest with space, so every slider
                            // starts at the same x (allocate_ui_with_layout
                            // shrinks to content width and does not reserve
                            // the 52px).
                            let label_resp = ui.label(
                                RichText::new(label)
                                    .size(13.0)
                                    .color(theme.subtext),
                            );
                            ui.add_space(
                                (52.0 - label_resp.rect.width()).max(0.0),
                            );
                            // The Slider does not expand to fill the row, so
                            // give it an explicit width: the remaining row
                            // width minus the 24px value box. Otherwise the
                            // track ends short and the value floats mid-row.
                            let slider_w = (ui.available_width() - 24.0).max(0.0);
                            let mut v = value as f32;
                            let resp = ui.add_sized(
                                [slider_w, 0.0],
                                egui::Slider::new(&mut v, 0.0..=100.0)
                                    .show_value(false)
                                    .trailing_fill(true),
                            );
                            let changed = resp.changed();
                            if !tip.is_empty() {
                                resp.on_hover_text(tip);
                            }
                            if changed {
                                set(&mut self.audio, v as u8);
                                self.save_settings();
                            }
                            // Right-align the value in its 24px box: measure
                            // the text and fill the space before it, so its
                            // right edge sits on the row's right edge.
                            let val_text = format!("{}", value);
                            let val_w = ui
                                .fonts(|f| {
                                    f.layout_no_wrap(
                                        val_text.clone(),
                                        egui::FontId::proportional(13.0),
                                        egui::Color32::BLACK,
                                    )
                                })
                                .size()
                                .x;
                            ui.add_space((24.0 - val_w).max(0.0));
                            ui.label(
                                RichText::new(val_text)
                                    .size(13.0)
                                    .color(theme.subtext),
                            );
                            ui.add_space(pad);
                        });
                    };
                    slider_row(ui, "Volume", vol, "", &mut |a, v| a.set_volume(v));
                    slider_row(
                        ui,
                        "Depth",
                        dep,
                        "How dark the sound is: higher cuts more highs",
                        &mut |a, v| a.set_depth(v),
                    );
                    slider_row(ui, "Rumble", rum, "Sub-bass boost", &mut |a, v| a.set_rumble(v));

                    // File row: label, file name, Choose button.
                    ui.horizontal(|ui| {
                        ui.add_space(pad);
                        // Fixed 52px label column, like the sliders:
                        // the file name must start at the slider column,
                        // not crammed against "File".
                        let file_resp = ui.label(
                            RichText::new("File")
                                .size(13.0)
                                .color(theme.subtext),
                        );
                        ui.add_space(
                            (52.0 - file_resp.rect.width()).max(0.0),
                        );
                        let fname = self
                            .file_path
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "None".to_string());
                        ui.add(
                            egui::Label::new(
                                RichText::new(fname).size(13.0).color(theme.subtext),
                            )
                            .truncate(),
                        );
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                ui.add_space(pad);
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("Choose…")
                                                .size(12.0)
                                                .color(theme.accent_text),
                                        )
                                        .frame(false),
                                    )
                                    .on_hover_text(
                                        "Loop a local audio file (wav, mp3, flac, ogg, m4a)",
                                    )
                                    .clicked()
                                {
                                    if self.choose_file() {
                                        self.selected = 5;
                                        if self.playing {
                                            self.play_selected();
                                        }
                                    }
                                }
                            },
                        );
                    });

                    // SLEEP TIMER (MIN).
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add_space(pad);
                        ui.label(
                            RichText::new("SLEEP TIMER (MIN)")
                                .font(semibold_font(11.0))
                                .color(theme.subtext),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.add_space(pad);
                        for (mins, label) in TIMER_CHOICES {
                            let selected = self.timer_minutes == mins;
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(label).size(12.0).color(
                                            if selected {
                                                theme.on_accent
                                            } else {
                                                theme.text
                                            },
                                        ),
                                    )
                                    .fill(if selected {
                                        theme.accent
                                    } else {
                                        theme.field
                                    })
                                    .corner_radius(Rounding::same(12))
                                    .min_size(egui::vec2(0.0, 28.0)),
                                )
                                .clicked()
                            {
                                self.timer_minutes = mins;
                                self.arm_timer();
                                self.save_settings();
                            }
                        }
                    });

                    // APPEARANCE.
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.add_space(pad);
                        ui.label(
                            RichText::new("APPEARANCE")
                                .font(semibold_font(11.0))
                                .color(theme.subtext),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.add_space(pad);
                        for mode in ThemeMode::ALL {
                            let selected = self.theme_mode == mode;
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(mode.label()).size(12.0).color(
                                            if selected {
                                                theme.on_accent
                                            } else {
                                                theme.text
                                            },
                                        ),
                                    )
                                    .fill(if selected {
                                        theme.accent
                                    } else {
                                        theme.field
                                    })
                                    .corner_radius(Rounding::same(12))
                                    .min_size(egui::vec2(0.0, 28.0)),
                                )
                                .clicked()
                            {
                                self.theme_mode = mode;
                                self.save_settings();
                            }
                        }
                    });

                    // Reduce motion.
                    ui.horizontal(|ui| {
                        ui.add_space(pad);
                        ui.label(
                            RichText::new("Reduce motion")
                                .size(13.0)
                                .color(theme.subtext),
                        );
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                ui.add_space(pad);
                                // right_to_left places the first item at the far right,
                                // so iterate On then Off to render "Off, On" left-to-right
                                // (Python Segmented order).
                                for (on, label) in [(true, "On"), (false, "Off")] {
                                    let selected = self.reduce_motion == on;
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new(label).size(12.0).color(
                                                    if selected {
                                                        theme.on_accent
                                                    } else {
                                                        theme.text
                                                    },
                                                ),
                                            )
                                            .fill(if selected {
                                                theme.accent
                                            } else {
                                                theme.field
                                            })
                                            .corner_radius(Rounding::same(12))
                                            .min_size(egui::vec2(0.0, 28.0)),
                                        )
                                        .clicked()
                                    {
                                        self.reduce_motion = on;
                                        self.save_settings();
                                    }
                                }
                            },
                        );
                    });
                }
            });
    }
}

/// A borderless square icon button using the real Lucide SVG
/// (same file and color as the Python app). Returns true on click.
/// `glyph_px` is the glyph size in points (Python's `icon_ratio`):
/// chevrons 18 in a 32px button, restart/gear 20 in a 40px button,
/// play/pause 26 in a 64px button.
///
/// Design-engineer details (Emil + better-ui + better-accessibility):
/// - Pressed state: Python fills the button with `field` while the
///   pointer is down, so the press reads instantly (Emil: a button
///   must feel responsive; press feedback at 100-160ms).
/// - Focus ring: Python draws a 2px `accent_text` ring on keyboard
///   focus; egui does not draw one for custom-painted widgets, so
///   this paints it when the widget has focus.
/// - Tooltip: the accessible name and the hover text, matching the
///   Python tooltips ("Previous sound", "Next sound", ...).
fn icon_button(
    ui: &mut egui::Ui,
    icons: &mut IconCache,
    kind: IconKind,
    button_px: f32,
    glyph_px: u32,
    color: Color32,
    theme: crate::theme::Theme,
    tooltip: &str,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(button_px), egui::Sense::click());
    let p = ui.painter_at(rect);
    // Pressed: field fill (Python `:pressed`), so the press reads.
    if resp.is_pointer_button_down_on() {
        p.rect_filled(rect, Rounding::same((button_px / 2.0) as u8), theme.field);
    } else if resp.hovered() {
        p.rect_filled(rect, Rounding::same((button_px / 2.0) as u8), theme.hover);
    }
    // Focus ring: 2px accent_text, like Python's `:focus` border.
    if resp.has_focus() {
        p.rect_stroke(
            rect,
            Rounding::same((button_px / 2.0) as u8),
            Stroke::new(2.0, theme.accent_text),
            egui::StrokeKind::Inside,
        );
    }
    if let Some(tex) = icons.texture(ui.ctx(), kind, color, glyph_px) {
        // Center the glyph in the button.
        let g = glyph_px as f32;
        let r = egui::Rect::from_center_size(rect.center(), egui::vec2(g, g));
        p.image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
    }
    resp.on_hover_text(tooltip).clicked()
}
