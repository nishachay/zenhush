//! Zenhush UI Window (`--ui`): lightweight on-demand Card & Notch Pill.
//!
//! Connects to the ~15 MB background Core over `BridgeClient`, renders
//! smoothly when open, and exits cleanly on hide/close to free GPU/Mesa RAM.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Color32, CornerRadius, Key, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::bridge::{BridgeClient, CoreToUi, UiCmd, UiSnapshot};
use crate::icons::{IconCache, IconKind};
use crate::settings::{DockAnchor, Settings, ThemeMode, WindowMode};
use crate::shortcuts;
use crate::synth::{FileTone, Sound};
use crate::theme::{Theme, DARK, LIGHT};

const CARD_W: f32 = 336.0;
const CARD_CLOSED_H: f32 = 344.0;
const CARD_OPEN_H: f32 = 602.0;
const NOTCH_W: f32 = 286.0;
const NOTCH_H: f32 = 42.0;

const BAR_COUNT: usize = 18;
const IDLE_BARS: [f32; BAR_COUNT] = [
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

pub struct ZenhushUiApp {
    client: BridgeClient,
    snap: UiSnapshot,
    icons: IconCache,
    drawer_open: bool,
    drawer_tab: DrawerTab,
    was_focused: bool,
    modal_open: bool,
    suppress_hide_until: Instant,
    anchor_applied: bool,
    last_window_size: [f32; 2],
    last_pos_change: Option<Instant>,
}

impl ZenhushUiApp {
    pub fn new(client: BridgeClient, settings: Settings) -> Self {
        let snap = UiSnapshot {
            playing: false,
            has_played: false,
            is_file_mode: settings.active_is_file || settings.locked_default_file.is_some(),
            active_sound: settings.active_sound,
            active_file: settings.locked_default_file.clone().or(settings.active_file),
            locked_default_file: settings.locked_default_file,
            local_tracks: settings.local_tracks,
            file_tone: settings.file_tone,
            loop_crossfade: settings.loop_crossfade,
            layer_sound: settings.layer_sound,
            layer_mix: settings.layer_mix,
            volume: settings.volume,
            muted: settings.muted,
            depth: settings.depth,
            rumble: settings.rumble,
            elapsed_secs: 0,
            timer_minutes: settings.timer_minutes,
            timer_remaining_secs: None,
            rms_bars: IDLE_BARS.to_vec(),
            audio_error: None,
            device_name: "System Default".to_string(),
            notice: None,
            missing_locked_file: None,
            theme_mode: settings.theme,
            reduce_motion: settings.reduce_motion,
            window_mode: settings.window_mode,
            dock_anchor: settings.dock_anchor,
            saved_pos: settings.saved_pos,
            pinned: settings.pinned,
            auto_play_on_launch: settings.auto_play_on_launch,
            start_in_background: settings.start_in_background,
            autostart_enabled: shortcuts::is_autostart_enabled(),
            gnome_pill_installed: false,
            global_shortcut_toggle: settings.global_shortcut_toggle,
            global_shortcut_play: settings.global_shortcut_play,
            shortcut_status: String::new(),
            has_tray: true,
        };

        ZenhushUiApp {
            client,
            snap,
            icons: IconCache::new(),
            drawer_open: false,
            drawer_tab: DrawerTab::Files,
            was_focused: false,
            modal_open: false,
            suppress_hide_until: Instant::now() + Duration::from_millis(450),
            anchor_applied: false,
            last_window_size: match settings.window_mode {
                WindowMode::NotchPill => [NOTCH_W, NOTCH_H],
                WindowMode::Card => [CARD_W, CARD_CLOSED_H],
            },
            last_pos_change: None,
        }
    }

    fn send(&self, cmd: UiCmd) {
        self.client.send(&cmd);
    }

    fn is_current_locked_default(&self) -> bool {
        self.snap.is_file_mode
            && self.snap.active_file.is_some()
            && self.snap.active_file == self.snap.locked_default_file
    }

    fn active_track_title(&self) -> String {
        let Some(path) = &self.snap.active_file else {
            return "Local File".to_string();
        };
        if let Some(t) = self.snap.local_tracks.iter().find(|t| &t.path == path) {
            return t.title.clone();
        }
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Local File".to_string())
    }

    fn display_title(&self) -> String {
        if self.snap.is_file_mode {
            if self.snap.active_file.is_some() {
                truncate_chars(&self.active_track_title(), 22)
            } else {
                "Local Audio".to_string()
            }
        } else {
            self.snap.active_sound.title().to_string()
        }
    }

    fn display_subtitle(&self) -> String {
        if !self.snap.playing && !self.snap.has_played {
            if self.is_current_locked_default() {
                return "locked default • press play".to_string();
            }
            return "press play to focus".to_string();
        }
        if self.snap.is_file_mode {
            if self.is_current_locked_default() {
                "locked default track".to_string()
            } else {
                "local audio loop".to_string()
            }
        } else if let Some(layer) = self.snap.layer_sound {
            format!(
                "{} + {}",
                self.snap.active_sound.subtitle(),
                layer.short_label().to_lowercase()
            )
        } else {
            self.snap.active_sound.subtitle().to_string()
        }
    }

    fn status_badge(&self) -> String {
        if self.snap.audio_error.is_some() {
            return "No audio device".to_string();
        }
        if let Some(rem) = self.snap.timer_remaining_secs {
            if self.snap.playing || rem > 0 {
                return format_duration(rem);
            }
        }
        if !self.snap.has_played {
            return if self.is_current_locked_default() {
                "LOCKED".to_string()
            } else {
                "READY".to_string()
            };
        }
        if !self.snap.playing {
            return "Paused".to_string();
        }
        if self.snap.is_file_mode {
            if self.snap.layer_sound.is_some() {
                "File + Mix".to_string()
            } else if self.is_current_locked_default() {
                "Locked File".to_string()
            } else {
                "File".to_string()
            }
        } else if self.snap.layer_sound.is_some() {
            "Layered".to_string()
        } else {
            "Live".to_string()
        }
    }

    fn target_size(&self) -> Vec2 {
        match self.snap.window_mode {
            WindowMode::NotchPill => Vec2::new(NOTCH_W, NOTCH_H),
            WindowMode::Card => {
                let mut h = if self.drawer_open {
                    CARD_OPEN_H
                } else {
                    CARD_CLOSED_H
                };
                if self.snap.notice.is_some() || self.snap.missing_locked_file.is_some() {
                    h += 28.0;
                }
                if !self.snap.has_tray {
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
            if self.snap.dock_anchor != DockAnchor::Free {
                self.anchor_applied = false;
            }
        }

        if !self.anchor_applied {
            if let Some(mon) = ctx.input(|i| i.viewport().monitor_size) {
                let pos = match self.snap.dock_anchor {
                    DockAnchor::TopCenter => {
                        let y = if self.snap.window_mode == WindowMode::NotchPill {
                            6.0
                        } else {
                            36.0
                        };
                        Some(Pos2::new(((mon.x - size.x) * 0.5).max(8.0), y))
                    }
                    DockAnchor::TopRight => {
                        Some(Pos2::new((mon.x - size.x - 16.0).max(8.0), 36.0))
                    }
                    DockAnchor::Free => self.snap.saved_pos.map(|[x, y]| Pos2::new(x, y)),
                };
                if let Some(p) = pos {
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(p));
                }
                self.anchor_applied = true;
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
            self.snap.active_file = Some(path.clone());
            self.snap.is_file_mode = true;
            if lock_after {
                self.snap.locked_default_file = Some(path.clone());
            }
            self.send(UiCmd::AddLocalFile {
                path,
                play_now: true,
                lock_default: lock_after,
            });
        }
    }

    fn toggle_notch_mode(&mut self, ctx: &egui::Context) {
        let next = match self.snap.window_mode {
            WindowMode::Card => WindowMode::NotchPill,
            WindowMode::NotchPill => WindowMode::Card,
        };
        self.snap.window_mode = next;
        self.anchor_applied = false;
        self.apply_window_geometry(ctx);
        self.send(UiCmd::SetWindowMode(next));
    }

    fn close_popup(&self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        ctx.request_repaint();
    }

    fn theme(&self, ctx: &egui::Context) -> Theme {
        match self.snap.theme_mode {
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

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
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
                0 => self.send(UiCmd::TogglePlay),
                1 => self.send(UiCmd::StepSound(-1)),
                2 => self.send(UiCmd::StepSound(1)),
                3 => {
                    let v = self.snap.volume.saturating_add(5).min(100);
                    self.snap.volume = v;
                    self.send(UiCmd::SetVolume(v));
                }
                30 => {
                    let v = self.snap.volume.saturating_add(1).min(100);
                    self.snap.volume = v;
                    self.send(UiCmd::SetVolume(v));
                }
                4 => {
                    let v = self.snap.volume.saturating_sub(5);
                    self.snap.volume = v;
                    self.send(UiCmd::SetVolume(v));
                }
                40 => {
                    let v = self.snap.volume.saturating_sub(1);
                    self.snap.volume = v;
                    self.send(UiCmd::SetVolume(v));
                }
                5 => self.send(UiCmd::ToggleMute),
                6 => self.send(UiCmd::Restart),
                7 => {
                    if let Some(p) = self.snap.active_file.clone() {
                        self.send(UiCmd::ToggleLockDefault(p));
                    }
                }
                8 => self.toggle_notch_mode(ctx),
                9 => {
                    self.snap.pinned = !self.snap.pinned;
                    self.send(UiCmd::SetPinned(self.snap.pinned));
                }
                10 => self.send(UiCmd::CycleTimer),
                11 => {
                    self.drawer_open = !self.drawer_open;
                    self.apply_window_geometry(ctx);
                }
                12 => {
                    if self.drawer_open {
                        self.drawer_open = false;
                        self.apply_window_geometry(ctx);
                    } else if !self.snap.pinned && self.snap.has_tray {
                        self.close_popup(ctx);
                    }
                }
                n if n >= 100 => {
                    let idx = (n - 100) as usize;
                    if idx < Sound::ALL.len() {
                        self.send(UiCmd::SelectSound(Sound::ALL[idx]));
                    } else if idx == Sound::ALL.len() && self.snap.active_file.is_some() {
                        self.send(UiCmd::SelectFileMode);
                    }
                }
                _ => {}
            }
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
            self.send(UiCmd::AddLocalFile {
                path,
                play_now: true,
                lock_default: false,
            });
        }
        hovering_file
    }

    fn bar_height_at(&self, i: usize) -> f32 {
        self.snap
            .rms_bars
            .get(i)
            .copied()
            .unwrap_or_else(|| IDLE_BARS[i % BAR_COUNT])
    }

    fn loudest_bar(&self) -> usize {
        let mut best = 0;
        let mut best_v = -1.0f32;
        for i in 0..BAR_COUNT {
            let v = self.bar_height_at(i);
            if v > best_v {
                best_v = v;
                best = i;
            }
        }
        best
    }

    fn render_notch_pill(&mut self, ctx: &egui::Context, theme: Theme) {
        let frame = egui::Frame::default()
            .fill(theme.card)
            .corner_radius(21.0)
            .stroke(Stroke::new(1.0_f32, theme.edge))
            .inner_margin(egui::Margin::symmetric(10, 6));

        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 1.5 && ui.rect_contains_pointer(ui.max_rect()) {
                let step = if scroll > 0.0 { 3i16 } else { -3i16 };
                let next = (self.snap.volume as i16 + step).clamp(0, 100) as u8;
                self.snap.volume = next;
                self.send(UiCmd::SetVolume(next));
            }

            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(7.0, 0.0);

                let (eq_rect, eq_resp) =
                    ui.allocate_exact_size(Vec2::new(34.0, 20.0), Sense::click_and_drag());
                if eq_resp.drag_started() {
                    self.snap.dock_anchor = DockAnchor::Free;
                    self.send(UiCmd::SetDockAnchor(DockAnchor::Free));
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                let p = ui.painter_at(eq_rect);
                let accent_slot = (self.loudest_bar() / 3).min(5);
                for i in 0..6 {
                    let h = (self.bar_height_at(i * 3) * 18.0).max(3.0);
                    let x = eq_rect.min.x + i as f32 * 5.6 + 1.0;
                    let y = eq_rect.center().y - h * 0.5;
                    let c = if self.snap.playing && i == accent_slot {
                        theme.accent
                    } else if self.snap.playing {
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

                let label_text = if let Some(rem) = self.snap.timer_remaining_secs {
                    if self.snap.playing && rem > 0 {
                        format!(
                            "{} • {}",
                            truncate_chars(&self.display_title(), 12),
                            format_duration(rem)
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
                    self.snap.dock_anchor = DockAnchor::Free;
                    self.send(UiCmd::SetDockAnchor(DockAnchor::Free));
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
                let glyph = if self.snap.playing {
                    IconKind::Pause
                } else {
                    IconKind::Play
                };
                if let Some(tex) = self.icons.texture(ctx, glyph, theme.on_accent, 13) {
                    let nudge = if self.snap.playing { 0.0 } else { 1.0 };
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
                    .on_hover_text(if self.snap.playing {
                        "Pause (Space)"
                    } else {
                        "Play (Space)"
                    })
                    .clicked()
                {
                    self.send(UiCmd::TogglePlay);
                }

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

            // --- 1. Top Micro-Bar ---
            ui.horizontal(|ui| {
                ui.set_height(24.0);
                let brand_color = if self.snap.playing {
                    theme.accent_text
                } else {
                    theme.subtext
                };
                let pill_text = if self.is_current_locked_default() {
                    "ZENHUSH • LOCKED"
                } else if self.snap.playing {
                    "ZENHUSH • ACTIVE"
                } else {
                    "ZENHUSH"
                };

                let drag_w = (ui.available_width() - 80.0).max(60.0);
                let (drag_rect, drag_resp) =
                    ui.allocate_exact_size(Vec2::new(drag_w, 22.0), Sense::click_and_drag());
                if drag_resp.drag_started() {
                    self.snap.dock_anchor = DockAnchor::Free;
                    self.send(UiCmd::SetDockAnchor(DockAnchor::Free));
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
                    if self.snap.has_tray
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
                        self.close_popup(ctx);
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
                        if self.snap.pinned {
                            theme.accent_text
                        } else {
                            theme.subtext
                        },
                        theme,
                        if self.snap.pinned {
                            "Pinned open (P)"
                        } else {
                            "Pin open (P)"
                        },
                    ) {
                        self.snap.pinned = !self.snap.pinned;
                        self.send(UiCmd::SetPinned(self.snap.pinned));
                    }
                });
            });

            ui.add_space(6.0);

            // --- 2. Visualizer Card ---
            let eq_w = ui.available_width();
            let (eq_rect, eq_resp) = ui.allocate_exact_size(Vec2::new(eq_w, 92.0), Sense::click());
            if eq_resp.double_clicked() {
                self.send(UiCmd::TogglePlay);
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
                let loudest = self.loudest_bar();

                for i in 0..BAR_COUNT {
                    let v = self.bar_height_at(i);
                    let h = (v * bar_h).max(3.0);
                    let x = eq_rect.min.x + margin + i as f32 * slot + (slot - bw) * 0.5;
                    let y = eq_rect.min.y + top + bar_h - h;
                    let color = if i == loudest && self.snap.playing {
                        theme.accent
                    } else if v > 0.62 {
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

                p.text(
                    Pos2::new(eq_rect.min.x + margin, eq_rect.max.y - 8.0),
                    egui::Align2::LEFT_BOTTOM,
                    format_duration(self.snap.elapsed_secs),
                    egui::FontId::proportional(11.5),
                    theme.subtext,
                );
                p.text(
                    Pos2::new(eq_rect.max.x - margin, eq_rect.max.y - 8.0),
                    egui::Align2::RIGHT_BOTTOM,
                    self.status_badge(),
                    semibold_font(11.5),
                    theme.accent_text,
                );
            }

            if let Some(missing) = self.snap.missing_locked_file.clone() {
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
                            self.send(UiCmd::UnlockMissingDefault);
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
            } else if let Some((msg, is_err)) = &self.snap.notice {
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

            // --- 3. Sound Header ---
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
                    self.send(UiCmd::StepSound(-1));
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
                    self.send(UiCmd::StepSound(1));
                }
            });

            ui.add_space(8.0);

            // --- 4. Quick Sound Chips ---
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                for &sound in &Sound::ALL {
                    let selected = !self.snap.is_file_mode && self.snap.active_sound == sound;
                    if pill_chip(ui, sound.short_label(), selected, theme) {
                        self.snap.active_sound = sound;
                        self.snap.is_file_mode = false;
                        self.send(UiCmd::SelectSound(sound));
                    }
                }
                let file_chip_label = if self.snap.locked_default_file.is_some() {
                    "🔒 File"
                } else {
                    "File"
                };
                if pill_chip(ui, file_chip_label, self.snap.is_file_mode, theme) {
                    if self.snap.active_file.is_none() && self.snap.local_tracks.is_empty() {
                        self.drawer_open = true;
                        self.drawer_tab = DrawerTab::Files;
                        self.apply_window_geometry(ctx);
                        self.pick_local_file(false);
                    } else {
                        self.snap.is_file_mode = true;
                        self.send(UiCmd::SelectFileMode);
                    }
                }
            });

            ui.add_space(10.0);

            // --- 5. Transport Row ---
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
                    self.send(UiCmd::Restart);
                }

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
                    pp.circle_stroke(
                        play_rect.center(),
                        24.5,
                        Stroke::new(2.0_f32, theme.accent_text),
                    );
                }
                let glyph = if self.snap.playing {
                    IconKind::Pause
                } else {
                    IconKind::Play
                };
                if let Some(tex) = self.icons.texture(ctx, glyph, theme.on_accent, 22) {
                    let nudge = if self.snap.playing { 0.0 } else { 1.5 };
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
                    .on_hover_text(if self.snap.playing {
                        "Pause (Space)"
                    } else {
                        "Play (Space)"
                    })
                    .clicked()
                {
                    self.snap.playing = !self.snap.playing;
                    self.send(UiCmd::TogglePlay);
                }

                let timer_txt = if self.snap.timer_minutes == 0 {
                    "Timer".to_string()
                } else {
                    format!("{}m", self.snap.timer_minutes)
                };
                let active_timer = self.snap.timer_minutes > 0;
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
                    self.send(UiCmd::CycleTimer);
                }

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

            // --- 6. Volume + Quick Lock Strip ---
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 0.0);
                let muted = self.snap.muted;
                if icon_button(
                    ui,
                    &mut self.icons,
                    if muted || self.snap.volume == 0 {
                        IconKind::Mute
                    } else {
                        IconKind::Volume
                    },
                    26.0,
                    16,
                    if muted {
                        theme.accent_text
                    } else {
                        theme.subtext
                    },
                    theme,
                    "Mute / Unmute (M)",
                ) {
                    self.snap.muted = !self.snap.muted;
                    self.send(UiCmd::ToggleMute);
                }

                let lock_btn_w = if self.snap.is_file_mode && self.snap.active_file.is_some() {
                    30.0
                } else {
                    0.0
                };
                let slider_w = (ui.available_width() - 36.0 - lock_btn_w).max(60.0);
                let mut vol_val = self.snap.volume as f32;
                let resp = ui.add_sized(
                    [slider_w, 20.0],
                    egui::Slider::new(&mut vol_val, 0.0..=100.0)
                        .show_value(false)
                        .trailing_fill(true),
                );
                if resp.hovered() {
                    let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                    if scroll.abs() > 1.0 {
                        let step = if scroll > 0.0 { 3.0 } else { -3.0 };
                        let next = (vol_val + step).clamp(0.0, 100.0) as u8;
                        self.snap.volume = next;
                        self.send(UiCmd::SetVolume(next));
                    }
                }
                if resp.changed() {
                    let next = vol_val as u8;
                    self.snap.volume = next;
                    self.send(UiCmd::SetVolume(next));
                }

                let vol_txt = if muted {
                    "Mute".to_string()
                } else {
                    format!("{}%", self.snap.volume)
                };
                ui.allocate_ui_with_layout(
                    Vec2::new(30.0, 20.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        ui.label(RichText::new(vol_txt).size(11.5).color(theme.subtext));
                    },
                );

                if self.snap.is_file_mode {
                    if let Some(path) = self.snap.active_file.clone() {
                        let locked = self.snap.locked_default_file.as_ref() == Some(&path);
                        if icon_button(
                            ui,
                            &mut self.icons,
                            if locked {
                                IconKind::Lock
                            } else {
                                IconKind::Unlock
                            },
                            26.0,
                            15,
                            if locked {
                                theme.accent_text
                            } else {
                                theme.subtext
                            },
                            theme,
                            if locked {
                                "Locked as default audio (L to unlock)"
                            } else {
                                "Lock this file as default audio (L)"
                            },
                        ) {
                            self.send(UiCmd::ToggleLockDefault(path));
                        }
                    }
                }
            });

            if !self.snap.has_tray && !self.drawer_open {
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
                        self.send(UiCmd::QuitDaemon);
                        self.close_popup(ctx);
                    }
                });
            }

            // --- 7. Tabbed Drawer ---
            if self.drawer_open {
                ui.add_space(10.0);
                ui.painter().line_segment(
                    [
                        Pos2::new(ui.min_rect().min.x, ui.cursor().min.y),
                        Pos2::new(
                            ui.min_rect().min.x + ui.available_width(),
                            ui.cursor().min.y,
                        ),
                    ],
                    Stroke::new(1.0_f32, theme.edge),
                );
                ui.add_space(8.0);

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

        let mut play_path: Option<PathBuf> = None;
        let mut lock_path: Option<PathBuf> = None;
        let mut remove_path: Option<PathBuf> = None;

        egui::ScrollArea::vertical()
            .max_height(104.0)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
                if self.snap.local_tracks.is_empty() {
                    ui.label(
                        RichText::new(
                            "No local tracks yet — add or drop wav, mp3, flac, ogg, m4a",
                        )
                        .size(11.5)
                        .color(theme.subtext),
                    );
                }
                for track in &self.snap.local_tracks {
                    let is_active = self.snap.is_file_mode
                        && self.snap.active_file.as_ref() == Some(&track.path);
                    let is_locked = self.snap.locked_default_file.as_ref() == Some(&track.path);
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
            self.send(UiCmd::AddLocalFile {
                path: p,
                play_now: true,
                lock_default: false,
            });
        }
        if let Some(p) = lock_path {
            self.send(UiCmd::ToggleLockDefault(p));
        }
        if let Some(p) = remove_path {
            self.send(UiCmd::RemoveLocalTrack(p));
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("Tone").size(12.0).color(theme.subtext));
            ui.add_space(6.0);
            for tone in FileTone::ALL {
                if pill_chip(ui, tone.label(), self.snap.file_tone == tone, theme) {
                    self.snap.file_tone = tone;
                    self.send(UiCmd::SetFileTone(tone));
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Loop seam").size(12.0).color(theme.subtext));
            ui.add_space(6.0);
            if pill_chip(ui, "Crossfade", self.snap.loop_crossfade, theme) {
                self.snap.loop_crossfade = true;
                self.send(UiCmd::SetLoopCrossfade(true));
            }
            if pill_chip(ui, "Gapless", !self.snap.loop_crossfade, theme) {
                self.snap.loop_crossfade = false;
                self.send(UiCmd::SetLoopCrossfade(false));
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
                    self.send(UiCmd::ResetControls);
                }
            });
        });

        let mut dep = self.snap.depth as f32;
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
                let v = dep as u8;
                self.snap.depth = v;
                self.send(UiCmd::SetDepth(v));
            }
            ui.label(
                RichText::new(format!("{}", self.snap.depth))
                    .size(12.0)
                    .color(theme.subtext),
            );
        });

        let mut rum = self.snap.rumble as f32;
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
                let v = rum as u8;
                self.snap.rumble = v;
                self.send(UiCmd::SetRumble(v));
            }
            ui.label(
                RichText::new(format!("{}", self.snap.rumble))
                    .size(12.0)
                    .color(theme.subtext),
            );
        });

        ui.add_space(2.0);
        ui.label(
            RichText::new("ATMOSPHERE LAYER")
                .font(semibold_font(10.5))
                .color(theme.subtext),
        );
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            if pill_chip(ui, "Off", self.snap.layer_sound.is_none(), theme) {
                self.snap.layer_sound = None;
                self.send(UiCmd::SetLayer(None));
            }
            for &ls in &Sound::ATMOSPHERE_LAYERS {
                if pill_chip(ui, ls.short_label(), self.snap.layer_sound == Some(ls), theme) {
                    self.snap.layer_sound = Some(ls);
                    self.send(UiCmd::SetLayer(Some(ls)));
                }
            }
        });
        if self.snap.layer_sound.is_some() {
            let mut mix = self.snap.layer_mix as f32;
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
                    let v = mix as u8;
                    self.snap.layer_mix = v;
                    self.send(UiCmd::SetLayerMix(v));
                }
                ui.label(
                    RichText::new(format!("{}%", self.snap.layer_mix))
                        .size(12.0)
                        .color(theme.subtext),
                );
            });
        }

        ui.add_space(2.0);
        ui.label(
            RichText::new("FOCUS / SLEEP TIMER (FADES OUT)")
                .font(semibold_font(10.5))
                .color(theme.subtext),
        );
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            for (mins, lbl) in TIMER_CHOICES {
                if pill_chip(ui, lbl, self.snap.timer_minutes == mins, theme) {
                    self.snap.timer_minutes = mins;
                    self.send(UiCmd::SetTimer(mins));
                }
            }
        });
    }

    fn render_tab_barkeys(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, theme: Theme) {
        ui.spacing_mut().item_spacing = Vec2::new(5.0, 6.0);

        ui.horizontal(|ui| {
            ui.label(RichText::new("Dock").size(12.0).color(theme.subtext));
            ui.add_space(4.0);
            for anchor in DockAnchor::ALL {
                if pill_chip(ui, anchor.label(), self.snap.dock_anchor == anchor, theme) {
                    self.snap.dock_anchor = anchor;
                    self.anchor_applied = false;
                    self.apply_window_geometry(ctx);
                    self.send(UiCmd::SetDockAnchor(anchor));
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label(RichText::new("Theme").size(12.0).color(theme.subtext));
            ui.add_space(4.0);
            for mode in ThemeMode::ALL {
                if pill_chip(ui, mode.label(), self.snap.theme_mode == mode, theme) {
                    self.snap.theme_mode = mode;
                    self.send(UiCmd::SetTheme(mode));
                }
            }
        });

        ui.horizontal(|ui| {
            if pill_chip(
                ui,
                "Auto-play on launch",
                self.snap.auto_play_on_launch,
                theme,
            ) {
                let v = !self.snap.auto_play_on_launch;
                self.snap.auto_play_on_launch = v;
                self.send(UiCmd::SetAutoPlay(v));
            }
            if pill_chip(ui, "Start in tray", self.snap.start_in_background, theme) {
                let v = !self.snap.start_in_background;
                self.snap.start_in_background = v;
                self.send(UiCmd::SetStartInBackground(v));
            }
            if pill_chip(ui, "Reduce motion", self.snap.reduce_motion, theme) {
                let v = !self.snap.reduce_motion;
                self.snap.reduce_motion = v;
                self.send(UiCmd::SetReduceMotion(v));
            }
        });

        #[cfg(target_os = "linux")]
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            if pill_chip(ui, "Launch at login", self.snap.autostart_enabled, theme) {
                let v = !self.snap.autostart_enabled;
                self.snap.autostart_enabled = v;
                self.send(UiCmd::SetAutostart(v));
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
                self.send(UiCmd::InstallLauncher);
            }
            if shortcuts::is_gnome() {
                let pill_lbl = if self.snap.gnome_pill_installed {
                    "GNOME Pill ✓"
                } else {
                    "Install GNOME Pill"
                };
                if pill_chip(ui, pill_lbl, self.snap.gnome_pill_installed, theme) {
                    self.send(UiCmd::ToggleGnomePill);
                }
                if ui
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
                    self.send(UiCmd::BindGnomeShortcut);
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label(RichText::new("Hotkey").size(11.5).color(theme.subtext));
            for preset in ["Alt+Shift+Z", "Ctrl+Shift+Z", "Super+Shift+Z"] {
                if pill_chip(
                    ui,
                    preset,
                    self.snap.global_shortcut_toggle == preset,
                    theme,
                ) {
                    self.snap.global_shortcut_toggle = preset.to_string();
                    self.send(UiCmd::SetGlobalShortcutToggle(preset.to_string()));
                }
            }
        });

        ui.label(
            RichText::new(format!(
                "Play: {}  |  {}",
                self.snap.global_shortcut_play, self.snap.shortcut_status
            ))
            .size(10.5)
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
            RichText::new(format!(
                "Output: {}",
                truncate_chars(&self.snap.device_name, 34)
            ))
            .size(10.5)
            .color(theme.subtext),
        );
    }
}

impl eframe::App for ZenhushUiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut msgs = Vec::new();
        while let Ok(m) = self.client.rx.try_recv() {
            msgs.push(m);
        }
        for m in msgs {
            match m {
                CoreToUi::Snapshot(s) => self.snap = s,
                CoreToUi::Focus => {
                    self.suppress_hide_until = Instant::now() + Duration::from_millis(350);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                CoreToUi::Close => {
                    self.close_popup(ctx);
                    return;
                }
            }
        }

        if self.snap.dock_anchor == DockAnchor::Free {
            if let Some(pos) = ctx.input(|i| i.viewport().outer_rect.map(|r| [r.min.x, r.min.y])) {
                if self.snap.saved_pos != Some(pos) {
                    self.snap.saved_pos = Some(pos);
                    self.last_pos_change = Some(Instant::now());
                }
            }
            if let Some(changed_at) = self.last_pos_change {
                if changed_at.elapsed() >= Duration::from_millis(500) {
                    self.last_pos_change = None;
                    self.send(UiCmd::SetSavedPos(self.snap.saved_pos));
                }
            }
        }

        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        if focused {
            self.was_focused = true;
        }
        let allow_hide = !self.snap.pinned
            && !self.modal_open
            && self.snap.window_mode == WindowMode::Card
            && Instant::now() >= self.suppress_hide_until
            && self.snap.has_tray;
        if allow_hide && self.was_focused && !focused {
            self.close_popup(ctx);
            return;
        }

        self.apply_window_geometry(ctx);
        self.handle_shortcuts(ctx);
        let hovering_file = self.handle_dropped_files(ctx);

        let theme = self.theme(ctx);
        let is_dark = self.snap.theme_mode == ThemeMode::Dark
            || (self.snap.theme_mode == ThemeMode::Auto
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

        if self.snap.playing || self.snap.timer_remaining_secs.is_some() || self.snap.notice.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(70));
        }

        match self.snap.window_mode {
            WindowMode::NotchPill => self.render_notch_pill(ctx, theme),
            WindowMode::Card => self.render_card(ctx, theme, hovering_file),
        }
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
