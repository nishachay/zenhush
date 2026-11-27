//! Lightweight Fallback Card UI (`--ui`).
//!
//! Used on macOS, Windows, non-GNOME Linux desktops, or on GNOME right after
//! initial installation before the Top-Bar Popover extension activates.
//!
//! Fixes & design engineering:
//! - Non-blocking async file picking (runs on background daemon thread so GNOME
//!   Wayland never shows "Is Not Responding") + Wayland `app_id("zenhush")`.
//! - Curved 16 px card corners on Wayland/macOS/Windows.
//! - Zero accidental scroll-wheel volume jumps.
//! - Dedicated Local Track + Lock default strip always visible from launch.
//! - Exact height computation so bottom content never clips.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Color32, CornerRadius, Key, Pos2, Rect, RichText, Sense, Stroke, Vec2};

use crate::bridge::{BridgeClient, CoreToUi, UiCmd, UiSnapshot};
use crate::icons::{IconCache, IconKind};
use crate::settings::{DockAnchor, Settings, ThemeMode, WindowMode};
use crate::shortcuts;
use crate::synth::Sound;
use crate::theme::{Theme, DARK, LIGHT};

const DEFAULT_CARD_W: f32 = 340.0;
const EDGE_RESIZE_PX: f32 = 6.0;
const BAR_COUNT: usize = 8;
const IDLE_BARS: [f32; BAR_COUNT] = [0.35, 0.60, 0.80, 0.55, 0.90, 0.65, 0.45, 0.30];
const TIMER_CYCLE: [(u32, &str); 5] = [
    (0, "Timer: Off"),
    (25, "Timer: 25m"),
    (45, "Timer: 45m"),
    (60, "Timer: 60m"),
    (90, "Timer: 90m"),
];

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

fn use_curved_transparent() -> bool {
    cfg!(not(target_os = "linux")) || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

pub struct ZenhushUiApp {
    client: BridgeClient,
    snap: UiSnapshot,
    icons: IconCache,
    was_focused: bool,
    suppress_hide_until: Instant,
    anchor_applied: bool,
    last_window_size: [f32; 2],
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
            window_mode: WindowMode::Card,
            dock_anchor: settings.dock_anchor,
            saved_pos: settings.saved_pos,
            pinned: settings.pinned,
            auto_play_on_launch: settings.auto_play_on_launch,
            start_in_background: settings.start_in_background,
            autostart_enabled: shortcuts::is_autostart_enabled(),
            gnome_pill_installed: false,
            gnome_popover_active: crate::gnome_ext::is_active(),
            picking_file: false,
            global_shortcut_toggle: settings.global_shortcut_toggle,
            global_shortcut_play: settings.global_shortcut_play,
            shortcut_status: String::new(),
            has_tray: true,
        };

        let init_h = Self::ideal_height(
            snap.active_file.is_some(),
            shortcuts::is_gnome() && !snap.gnome_popover_active,
        );

        ZenhushUiApp {
            client,
            snap,
            icons: IconCache::new(),
            was_focused: false,
            suppress_hide_until: Instant::now() + Duration::from_millis(450),
            anchor_applied: false,
            last_window_size: [DEFAULT_CARD_W, init_h],
        }
    }

    fn ideal_height(has_file: bool, gnome_hint: bool) -> f32 {
        let mut h = if has_file { 398.0 } else { 354.0 };
        if gnome_hint {
            h += 28.0;
        }
        h
    }

    fn send(&self, cmd: UiCmd) {
        self.client.send(&cmd);
    }

    fn poll_core(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.client.rx.try_recv() {
            match msg {
                CoreToUi::Snapshot(s) => {
                    if self.snap.picking_file && !s.picking_file {
                        self.suppress_hide_until = Instant::now() + Duration::from_millis(500);
                    }
                    self.snap = s;
                }
                CoreToUi::Focus => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                CoreToUi::Close => {
                    self.close_popup(ctx);
                }
            }
        }
    }

    fn is_locked_default(&self) -> bool {
        self.snap.active_file.is_some()
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

    fn full_title(&self) -> String {
        if self.snap.is_file_mode {
            self.active_track_title()
        } else {
            self.snap.active_sound.title().to_string()
        }
    }

    fn display_subtitle(&self) -> String {
        if let Some(ref err) = self.snap.audio_error {
            return truncate_chars(err, 28);
        }
        if !self.snap.playing && !self.snap.has_played {
            return if self.snap.is_file_mode && self.is_locked_default() {
                "locked default · ready".to_string()
            } else {
                "ready to focus".to_string()
            };
        }
        let state = if self.snap.playing { "playing" } else { "paused" };
        if self.snap.is_file_mode {
            if self.is_locked_default() {
                format!("locked default · {}", state)
            } else {
                format!("local track · {}", state)
            }
        } else {
            format!("{} · {}", self.snap.active_sound.subtitle(), state)
        }
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

    fn apply_window_geometry(&mut self, ctx: &egui::Context) {
        let cur_rect = ctx.input(|i| i.viewport().inner_rect);
        let cur_w = cur_rect.map(|r| r.width()).unwrap_or(self.last_window_size[0]);
        let cur_h = cur_rect.map(|r| r.height()).unwrap_or(self.last_window_size[1]);
        let need_h = Self::ideal_height(
            self.snap.active_file.is_some(),
            shortcuts::is_gnome() && !self.snap.gnome_popover_active,
        );

        // Expand cleanly if new content appears (or initial ideal height increased),
        // without overriding user's manual width resize.
        if (need_h - self.last_window_size[1]).abs() > 0.5 && cur_h + 1.0 < need_h {
            self.last_window_size = [cur_w.max(DEFAULT_CARD_W), need_h];
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                self.last_window_size[0],
                need_h,
            )));
            if self.snap.dock_anchor != DockAnchor::Free {
                self.anchor_applied = false;
            }
        } else {
            self.last_window_size[0] = cur_w;
        }

        if !self.anchor_applied {
            if let Some(mon) = ctx.input(|i| i.viewport().monitor_size) {
                let w = cur_w.max(DEFAULT_CARD_W);
                let pos = match self.snap.dock_anchor {
                    DockAnchor::TopCenter => Some(Pos2::new(((mon.x - w) * 0.5).max(8.0), 36.0)),
                    DockAnchor::TopRight => Some(Pos2::new((mon.x - w - 16.0).max(8.0), 36.0)),
                    DockAnchor::Free => self.snap.saved_pos.map(|[x, y]| Pos2::new(x, y)),
                };
                if let Some(p) = pos {
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(p));
                }
                self.anchor_applied = true;
            }
        }
    }

    fn handle_edge_resize(&mut self, ctx: &egui::Context) {
        let Some(rect) = ctx.input(|i| i.viewport().inner_rect) else {
            return;
        };
        let size = rect.size();
        let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        let left = pos.x <= EDGE_RESIZE_PX;
        let right = pos.x >= size.x - EDGE_RESIZE_PX;
        let top = pos.y <= EDGE_RESIZE_PX;
        let bottom = pos.y >= size.y - EDGE_RESIZE_PX;

        let dir = match (left, right, top, bottom) {
            (true, false, true, false) => Some((egui::ResizeDirection::NorthWest, egui::CursorIcon::ResizeNwSe)),
            (false, true, true, false) => Some((egui::ResizeDirection::NorthEast, egui::CursorIcon::ResizeNeSw)),
            (true, false, false, true) => Some((egui::ResizeDirection::SouthWest, egui::CursorIcon::ResizeNeSw)),
            (false, true, false, true) => Some((egui::ResizeDirection::SouthEast, egui::CursorIcon::ResizeNwSe)),
            (true, false, false, false) => Some((egui::ResizeDirection::West, egui::CursorIcon::ResizeHorizontal)),
            (false, true, false, false) => Some((egui::ResizeDirection::East, egui::CursorIcon::ResizeHorizontal)),
            (false, false, true, false) => Some((egui::ResizeDirection::North, egui::CursorIcon::ResizeVertical)),
            (false, false, false, true) => Some((egui::ResizeDirection::South, egui::CursorIcon::ResizeVertical)),
            _ => None,
        };

        if let Some((resize_dir, cursor)) = dir {
            ctx.set_cursor_icon(cursor);
            if ctx.input(|i| i.pointer.primary_pressed()) {
                self.snap.dock_anchor = DockAnchor::Free;
                self.send(UiCmd::SetDockAnchor(DockAnchor::Free));
                ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(resize_dir));
            }
        }
    }

    fn close_popup(&self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        ctx.request_repaint();
    }

    fn pick_local_file_async(&mut self, lock_default: bool) {
        self.suppress_hide_until = Instant::now() + Duration::from_secs(60);
        self.snap.picking_file = true;
        self.send(UiCmd::PickFile { lock_default });
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.wants_keyboard_input() {
            return;
        }
        let mut actions: Vec<i32> = Vec::new();
        ctx.input(|i| {
            if i.key_pressed(Key::Space) || i.key_pressed(Key::K) {
                actions.push(0);
            }
            if i.key_pressed(Key::ArrowLeft) {
                actions.push(1);
            }
            if i.key_pressed(Key::ArrowRight) {
                actions.push(2);
            }
            if i.key_pressed(Key::ArrowUp) || i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals)
            {
                actions.push(3);
            }
            if i.key_pressed(Key::ArrowDown) || i.key_pressed(Key::Minus) {
                actions.push(4);
            }
            if i.key_pressed(Key::M) {
                actions.push(5);
            }
            if i.key_pressed(Key::T) {
                actions.push(6);
            }
            if i.key_pressed(Key::O) {
                actions.push(8);
            }
            if i.key_pressed(Key::L) {
                actions.push(9);
            }
            if i.key_pressed(Key::P) {
                actions.push(10);
            }
            if i.key_pressed(Key::Escape) {
                actions.push(7);
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
                4 => {
                    let v = self.snap.volume.saturating_sub(5);
                    self.snap.volume = v;
                    self.send(UiCmd::SetVolume(v));
                }
                5 => self.send(UiCmd::ToggleMute),
                6 => self.send(UiCmd::CycleTimer),
                7 => {
                    if !self.snap.pinned && self.snap.has_tray {
                        self.close_popup(ctx);
                    }
                }
                8 => self.pick_local_file_async(false),
                9 => {
                    if let Some(p) = self.snap.active_file.clone() {
                        self.send(UiCmd::ToggleLockDefault(p));
                    } else {
                        self.pick_local_file_async(true);
                    }
                }
                10 => {
                    self.snap.pinned = !self.snap.pinned;
                    self.send(UiCmd::SetPinned(self.snap.pinned));
                }
                n if n >= 100 => {
                    let idx = (n - 100) as usize;
                    if let Some(&s) = Sound::ALL.get(idx) {
                        self.send(UiCmd::SelectSound(s));
                    }
                }
                _ => {}
            }
        }
    }

    fn handle_dropped_files(&self, ctx: &egui::Context) {
        let mut dropped: Option<PathBuf> = None;
        ctx.input(|i| {
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
    }

    fn bar_height(&self, i: usize) -> f32 {
        self.snap
            .rms_bars
            .get(i)
            .copied()
            .unwrap_or(IDLE_BARS[i % BAR_COUNT])
    }

    fn render_brand_glyph(&self, painter: &egui::Painter, center: Pos2, color: Color32) {
        let specs = [(-4.0_f32, 6.0_f32), (0.0, 11.0), (4.0, 8.0)];
        for (dx, h) in specs {
            painter.rect_filled(
                Rect::from_center_size(Pos2::new(center.x + dx, center.y), Vec2::new(2.2, h)),
                CornerRadius::same(1),
                color,
            );
        }
    }

    fn render_card(&mut self, ctx: &egui::Context, theme: Theme) {
        let card_radius = if use_curved_transparent() {
            16.0
        } else {
            0.0
        };
        let frame = egui::Frame::default()
            .fill(theme.card)
            .corner_radius(card_radius)
            .stroke(Stroke::new(1.0_f32, theme.edge))
            .inner_margin(egui::Margin::same(14));

        egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                .show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let avail_w = ui.available_width();

            // 1. Top Bar (28px height, 28x28 hit targets)
            ui.horizontal(|ui| {
                ui.set_height(28.0);
                let brand_w = (avail_w - 60.0).max(60.0);
                let (drag_rect, drag_resp) =
                    ui.allocate_exact_size(Vec2::new(brand_w, 28.0), Sense::click_and_drag());
                if drag_resp.drag_started() {
                    self.snap.dock_anchor = DockAnchor::Free;
                    self.send(UiCmd::SetDockAnchor(DockAnchor::Free));
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                let dp = ui.painter_at(drag_rect);
                let glyph_col = if self.snap.playing {
                    theme.accent
                } else {
                    theme.subtext
                };
                self.render_brand_glyph(
                    &dp,
                    Pos2::new(drag_rect.min.x + 6.0, drag_rect.center().y),
                    glyph_col,
                );
                dp.text(
                    Pos2::new(drag_rect.min.x + 17.0, drag_rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    if self.snap.playing {
                        "ZENHUSH · LIVE"
                    } else {
                        "ZENHUSH"
                    },
                    semibold_font(11.0),
                    if self.snap.playing {
                        theme.accent_text
                    } else {
                        theme.subtext
                    },
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(4.0, 0.0);
                    let close_tip = if self.snap.has_tray {
                        "Hide to top bar (Esc)"
                    } else {
                        "Quit Zenhush"
                    };
                    if icon_button(
                        ui,
                        &mut self.icons,
                        IconKind::Close,
                        28.0,
                        14,
                        theme.subtext,
                        theme,
                        close_tip,
                    ) {
                        self.close_popup(ctx);
                    }
                    let pin_color = if self.snap.pinned {
                        theme.accent_text
                    } else {
                        theme.subtext
                    };
                    if icon_button(
                        ui,
                        &mut self.icons,
                        IconKind::Pin,
                        28.0,
                        14,
                        pin_color,
                        theme,
                        if self.snap.pinned {
                            "Unpin window (P)"
                        } else {
                            "Pin window open (P)"
                        },
                    ) {
                        self.snap.pinned = !self.snap.pinned;
                        self.send(UiCmd::SetPinned(self.snap.pinned));
                    }
                });
            });

            ui.add_space(12.0);

            // 2. Hero Stage: Recessed Warm Container (68px tall, 44x44 Play target)
            let (hero_rect, _) =
                ui.allocate_exact_size(Vec2::new(avail_w, 68.0), Sense::hover());
            let hp = ui.painter_at(hero_rect);
            hp.rect_filled(hero_rect, CornerRadius::same(12), theme.inset);

            let mut hero_ui = ui.new_child(
                egui::UiBuilder::new().max_rect(hero_rect.shrink2(Vec2::new(12.0, 10.0))),
            );
            hero_ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                let inner_w = ui.available_width();
                let text_w = inner_w - 54.0;
                let full_name = self.full_title();

                ui.allocate_ui(Vec2::new(text_w, 48.0), |ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::ZERO;
                        let max_chars = ((text_w / 8.8).round() as usize).clamp(14, 42);
                        let shown_title = truncate_chars(&full_name, max_chars);
                        let title_resp = ui.label(
                            RichText::new(&shown_title)
                                .font(bold_font(16.0))
                                .color(theme.text),
                        );
                        if full_name != shown_title {
                            title_resp.on_hover_text(&full_name);
                        }
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing = Vec2::new(7.0, 0.0);
                            let (eq_rect, _) =
                                ui.allocate_exact_size(Vec2::new(34.0, 14.0), Sense::hover());
                            let ep = ui.painter_at(eq_rect);
                            let bw = 2.4_f32;
                            let gap = 1.8_f32;
                            for i in 0..BAR_COUNT {
                                let h = (self.bar_height(i) * 12.0).max(2.5);
                                let x = eq_rect.min.x + i as f32 * (bw + gap);
                                let y = eq_rect.center().y - h * 0.5;
                                ep.rect_filled(
                                    Rect::from_min_size(Pos2::new(x, y), Vec2::new(bw, h)),
                                    CornerRadius::same(1),
                                    if self.snap.playing {
                                        theme.accent
                                    } else {
                                        theme.bar
                                    },
                                );
                            }
                            let sub_color = if self.snap.audio_error.is_some() {
                                theme.accent_text
                            } else {
                                theme.subtext
                            };
                            let sub_resp = ui.label(
                                RichText::new(self.display_subtitle())
                                    .font(semibold_font(12.0))
                                    .color(sub_color),
                            );
                            if let Some(ref err) = self.snap.audio_error {
                                sub_resp.on_hover_text(err);
                            }
                        });
                    });
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (play_rect, play_resp) =
                        ui.allocate_exact_size(Vec2::splat(44.0), Sense::click());
                    let pp = ui.painter_at(play_rect);
                    let fill = if play_resp.is_pointer_button_down_on() {
                        theme.pressed
                    } else if play_resp.hovered() {
                        theme.accent.gamma_multiply(0.92)
                    } else {
                        theme.accent
                    };
                    pp.circle_filled(play_rect.center(), 22.0, fill);
                    if play_resp.has_focus() {
                        pp.circle_stroke(
                            play_rect.center(),
                            21.0,
                            Stroke::new(2.0_f32, theme.text),
                        );
                    }
                    let glyph = if self.snap.playing {
                        IconKind::Pause
                    } else {
                        IconKind::Play
                    };
                    if let Some(tex) = self.icons.texture(ctx, glyph, theme.on_accent, 18) {
                        let nudge = if self.snap.playing { 0.0 } else { 1.5 };
                        pp.image(
                            tex.id(),
                            Rect::from_center_size(
                                play_rect.center() + Vec2::new(nudge, 0.0),
                                Vec2::splat(18.0),
                            ),
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    if play_resp
                        .on_hover_text(if self.snap.playing {
                            "Pause focus audio (Space)"
                        } else {
                            "Play focus audio (Space)"
                        })
                        .clicked()
                    {
                        self.send(UiCmd::TogglePlay);
                    }
                });
            });

            ui.add_space(14.0);

            // 3. Responsive Sound Grid (4 columns normally, adapts to 2 columns if narrowed)
            let cols = if avail_w >= 260.0 { 4 } else { 2 };
            let rows = (Sound::ALL.len() + cols - 1) / cols;
            let chip_gap = 6.0;
            let chip_w = (avail_w - chip_gap * (cols as f32 - 1.0)) / cols as f32;
            for row in 0..rows {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(chip_gap, 0.0);
                    for col in 0..cols {
                        let idx = row * cols + col;
                        let Some(&sound) = Sound::ALL.get(idx) else {
                            continue;
                        };
                        let active = !self.snap.is_file_mode && self.snap.active_sound == sound;
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::new(chip_w, 32.0), Sense::click());
                        let bg = if active {
                            theme.accent
                        } else if resp.hovered() {
                            theme.hover
                        } else {
                            theme.inset
                        };
                        let fg = if active { theme.on_accent } else { theme.text };
                        let p = ui.painter_at(rect);
                        p.rect_filled(rect, CornerRadius::same(8), bg);
                        if resp.has_focus() {
                            p.rect_stroke(
                                rect,
                                CornerRadius::same(8),
                                Stroke::new(2.0_f32, theme.text),
                                egui::StrokeKind::Inside,
                            );
                        } else if active {
                            p.rect_stroke(
                                rect,
                                CornerRadius::same(8),
                                Stroke::new(1.5_f32, theme.on_accent.gamma_multiply(0.35)),
                                egui::StrokeKind::Inside,
                            );
                        }
                        p.text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            sound.short_label(),
                            semibold_font(12.0),
                            fg,
                        );
                        if resp.clicked() {
                            self.send(UiCmd::SelectSound(sound));
                        }
                    }
                });
                if row + 1 < rows {
                    ui.add_space(6.0);
                }
            }

            ui.add_space(14.0);

            // 4. Volume Row (28px tall, unclipped knob, zero accidental scroll volume)
            ui.horizontal(|ui| {
                ui.set_height(28.0);
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 0.0);
                let mute_kind = if self.snap.muted || self.snap.volume == 0 {
                    IconKind::Mute
                } else {
                    IconKind::Volume
                };
                let mute_col = if self.snap.muted {
                    theme.accent_text
                } else {
                    theme.subtext
                };
                if icon_button(
                    ui,
                    &mut self.icons,
                    mute_kind,
                    28.0,
                    16,
                    mute_col,
                    theme,
                    if self.snap.muted {
                        "Unmute audio (M)"
                    } else {
                        "Mute audio (M)"
                    },
                ) {
                    self.send(UiCmd::ToggleMute);
                }

                let slider_w = (avail_w - 28.0 - 40.0 - 12.0).max(80.0);
                let (s_rect, s_resp) =
                    ui.allocate_exact_size(Vec2::new(slider_w, 28.0), Sense::click_and_drag());
                let knob_r = 7.0_f32;
                let track_left = s_rect.min.x + knob_r;
                let track_w = (s_rect.width() - knob_r * 2.0).max(1.0);
                if s_resp.clicked() || s_resp.dragged() {
                    if let Some(pos) = s_resp.interact_pointer_pos() {
                        let frac = ((pos.x - track_left) / track_w).clamp(0.0, 1.0);
                        let next = (frac * 100.0).round() as u8;
                        if next != self.snap.volume || self.snap.muted {
                            self.snap.volume = next;
                            self.snap.muted = false;
                            self.send(UiCmd::SetVolume(next));
                        }
                    }
                }
                let sp = ui.painter_at(s_rect);
                let track = Rect::from_min_size(
                    Pos2::new(track_left, s_rect.center().y - 2.5),
                    Vec2::new(track_w, 5.0),
                );
                sp.rect_filled(track, CornerRadius::same(3), theme.inset);
                let shown_vol = if self.snap.muted { 0 } else { self.snap.volume };
                let fill_w = track_w * (shown_vol as f32 / 100.0);
                if fill_w > 1.0 {
                    let fill = Rect::from_min_size(track.min, Vec2::new(fill_w, track.height()));
                    sp.rect_filled(fill, CornerRadius::same(3), theme.accent);
                }
                let knob_x = track_left + fill_w;
                sp.circle_filled(
                    Pos2::new(knob_x, track.center().y),
                    knob_r,
                    if self.snap.muted {
                        theme.subtext
                    } else {
                        theme.accent
                    },
                );
                sp.circle_stroke(
                    Pos2::new(knob_x, track.center().y),
                    knob_r,
                    Stroke::new(1.5_f32, theme.card),
                );

                let vol_txt = if self.snap.muted {
                    "Mute".to_string()
                } else {
                    format!("{}%", self.snap.volume)
                };
                let (pct_rect, _) = ui.allocate_exact_size(Vec2::new(40.0, 28.0), Sense::hover());
                ui.painter_at(pct_rect).text(
                    Pos2::new(pct_rect.max.x, pct_rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    vol_txt,
                    semibold_font(12.0),
                    theme.subtext,
                );
            });

            ui.add_space(12.0);

            // 5. Actions Row: Sleep Timer + Choose File… (32px tall, non-blocking async file dialog)
            let pill_gap = 8.0;
            let pill_w = (avail_w - pill_gap) * 0.5;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::new(pill_gap, 0.0);

                let timer_txt = if let Some(rem) = self.snap.timer_remaining_secs {
                    if rem > 0 {
                        format!("Timer: {}", format_duration(rem))
                    } else {
                        "Timer: Off".to_string()
                    }
                } else if self.snap.timer_minutes > 0 {
                    format!("Timer: {}m", self.snap.timer_minutes)
                } else {
                    "Timer: Off".to_string()
                };
                let timer_on = self.snap.timer_minutes > 0;
                let (t_rect, t_resp) =
                    ui.allocate_exact_size(Vec2::new(pill_w, 32.0), Sense::click());
                let tp = ui.painter_at(t_rect);
                tp.rect_filled(
                    t_rect,
                    CornerRadius::same(8),
                    if timer_on {
                        theme.accent
                    } else if t_resp.hovered() {
                        theme.hover
                    } else {
                        theme.inset
                    },
                );
                if t_resp.has_focus() {
                    tp.rect_stroke(
                        t_rect,
                        CornerRadius::same(8),
                        Stroke::new(2.0_f32, theme.text),
                        egui::StrokeKind::Inside,
                    );
                }
                tp.text(
                    t_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    timer_txt,
                    semibold_font(12.0),
                    if timer_on {
                        theme.on_accent
                    } else {
                        theme.text
                    },
                );
                if t_resp.on_hover_text("Cycle sleep timer (T)").clicked() {
                    let cur = self.snap.timer_minutes;
                    let idx = TIMER_CYCLE.iter().position(|&(m, _)| m == cur).unwrap_or(0);
                    let next = TIMER_CYCLE[(idx + 1) % TIMER_CYCLE.len()].0;
                    self.snap.timer_minutes = next;
                    self.send(UiCmd::SetTimer(next));
                }

                let (f_rect, f_resp) =
                    ui.allocate_exact_size(Vec2::new(pill_w, 32.0), Sense::click());
                let fp = ui.painter_at(f_rect);
                fp.rect_filled(
                    f_rect,
                    CornerRadius::same(8),
                    if f_resp.hovered() {
                        theme.hover
                    } else {
                        theme.inset
                    },
                );
                if f_resp.has_focus() {
                    fp.rect_stroke(
                        f_rect,
                        CornerRadius::same(8),
                        Stroke::new(2.0_f32, theme.text),
                        egui::StrokeKind::Inside,
                    );
                }
                fp.text(
                    f_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    if self.snap.picking_file {
                        "Choosing…"
                    } else if self.snap.active_file.is_some() {
                        "Change File…"
                    } else {
                        "+ My Audio…"
                    },
                    semibold_font(12.0),
                    theme.text,
                );
                if f_resp
                    .on_hover_text("Choose or drop an audio file — automatically remembered on launch (O)")
                    .clicked()
                {
                    self.pick_local_file_async(false);
                }
            });

            // 6. Clean Local Track Pill (only shown when a local file is loaded — zero empty box clutter)
            if let Some(file_path) = self.snap.active_file.clone() {
                ui.add_space(10.0);
                let (row_rect, _) = ui.allocate_exact_size(Vec2::new(avail_w, 34.0), Sense::hover());
                let rp = ui.painter_at(row_rect);
                rp.rect_filled(
                    row_rect,
                    CornerRadius::same(9),
                    if self.snap.is_file_mode {
                        theme.accent.gamma_multiply(0.14)
                    } else {
                        theme.inset
                    },
                );
                if self.snap.is_file_mode {
                    rp.rect_stroke(
                        row_rect,
                        CornerRadius::same(9),
                        Stroke::new(1.0_f32, theme.accent.gamma_multiply(0.55)),
                        egui::StrokeKind::Inside,
                    );
                }

                let mut child_ui = ui.new_child(
                    egui::UiBuilder::new().max_rect(row_rect.shrink2(Vec2::new(8.0, 4.0))),
                );
                child_ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(4.0, 0.0);
                    let locked = self.is_locked_default();
                    let lock_w = if locked { 72.0 } else { 68.0 };
                    let remove_w = 26.0;
                    let name_w = (ui.available_width() - lock_w - remove_w - 8.0).max(50.0);

                    let full_track = self.active_track_title();
                    let max_track_chars = ((name_w / 7.2).round() as usize).clamp(10, 32);
                    let shown_track = truncate_chars(&full_track, max_track_chars);

                    let (n_rect, n_resp) =
                        ui.allocate_exact_size(Vec2::new(name_w, 26.0), Sense::click());
                    let np = ui.painter_at(n_rect);
                    let name_col = if self.snap.is_file_mode {
                        theme.accent_text
                    } else {
                        theme.text
                    };
                    if let Some(tex) = self.icons.texture(ui.ctx(), IconKind::Music, name_col, 13) {
                        np.image(
                            tex.id(),
                            Rect::from_center_size(
                                Pos2::new(n_rect.min.x + 8.0, n_rect.center().y),
                                Vec2::splat(13.0),
                            ),
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    np.text(
                        Pos2::new(n_rect.min.x + 20.0, n_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        shown_track,
                        semibold_font(12.0),
                        name_col,
                    );
                    if n_resp
                        .on_hover_text(format!("Play {}", full_track))
                        .clicked()
                    {
                        self.send(UiCmd::SelectFileMode);
                    }

                    // Flat inline Lock Startup Default capsule pill (clean icon + "Default")
                    let (l_rect, l_resp) =
                        ui.allocate_exact_size(Vec2::new(lock_w, 24.0), Sense::click());
                    let lp = ui.painter_at(l_rect);
                    let lock_bg = if locked {
                        theme.accent
                    } else if l_resp.hovered() {
                        theme.hover
                    } else {
                        Color32::TRANSPARENT
                    };
                    if lock_bg != Color32::TRANSPARENT {
                        lp.rect_filled(l_rect, CornerRadius::same(6), lock_bg);
                    }
                    if l_resp.has_focus() {
                        lp.rect_stroke(
                            l_rect,
                            CornerRadius::same(6),
                            Stroke::new(2.0_f32, theme.text),
                            egui::StrokeKind::Inside,
                        );
                    }
                    let lock_fg = if locked {
                        theme.on_accent
                    } else {
                        theme.subtext
                    };
                    let lock_icon = if locked {
                        IconKind::Lock
                    } else {
                        IconKind::Unlock
                    };
                    if let Some(tex) = self.icons.texture(ui.ctx(), lock_icon, lock_fg, 12) {
                        lp.image(
                            tex.id(),
                            Rect::from_center_size(
                                Pos2::new(l_rect.min.x + 12.0, l_rect.center().y),
                                Vec2::splat(12.0),
                            ),
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                    lp.text(
                        Pos2::new(l_rect.min.x + 22.0, l_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        "Default",
                        semibold_font(11.0),
                        lock_fg,
                    );
                    let lock_tip = if locked {
                        "Locked: always opens Zenhush with this track on launch — click to unlock (L)"
                    } else {
                        "Lock as startup default (even if you later play Brown Noise) [L]"
                    };
                    if l_resp.on_hover_text(lock_tip).clicked() {
                        self.send(UiCmd::ToggleLockDefault(file_path.clone()));
                    }

                    if icon_button(
                        ui,
                        &mut self.icons,
                        IconKind::Close,
                        24.0,
                        12,
                        theme.subtext,
                        theme,
                        "Remove local track",
                    ) {
                        self.send(UiCmd::RemoveLocalTrack(file_path));
                    }
                });
            }

            // 7. GNOME activation notice (wrapped safely if narrow)
            if shortcuts::is_gnome() && !self.snap.gnome_popover_active {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Log out & back in once for top-bar popover")
                        .font(semibold_font(11.0))
                        .color(theme.subtext),
                );
            }
                });
        });
    }
}

impl eframe::App for ZenhushUiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_core(ctx);

        let focused = ctx.input(|i| i.focused);
        if !focused
            && self.was_focused
            && !self.snap.pinned
            && self.snap.has_tray
            && !self.snap.picking_file
            && Instant::now() >= self.suppress_hide_until
        {
            self.close_popup(ctx);
        }
        self.was_focused = focused;

        self.handle_edge_resize(ctx);
        self.handle_shortcuts(ctx);
        self.handle_dropped_files(ctx);
        self.apply_window_geometry(ctx);

        let theme = self.theme(ctx);
        self.render_card(ctx, theme);

        if self.snap.playing || self.snap.timer_remaining_secs.is_some() || self.snap.picking_file {
            ctx.request_repaint_after(Duration::from_millis(250));
        } else {
            ctx.request_repaint_after(Duration::from_secs(5));
        }
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        if use_curved_transparent() {
            [0.0, 0.0, 0.0, 0.0]
        } else {
            let bg = if visuals.dark_mode {
                DARK.card
            } else {
                LIGHT.card
            };
            [
                bg.r() as f32 / 255.0,
                bg.g() as f32 / 255.0,
                bg.b() as f32 / 255.0,
                1.0,
            ]
        }
    }
}

fn icon_button(
    ui: &mut egui::Ui,
    icons: &mut IconCache,
    kind: IconKind,
    size: f32,
    icon_px: u32,
    color: Color32,
    theme: Theme,
    tooltip: &str,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let bg = if resp.is_pointer_button_down_on() {
        theme.pressed.gamma_multiply(0.35)
    } else if resp.hovered() {
        theme.hover
    } else {
        Color32::TRANSPARENT
    };
    let p = ui.painter_at(rect);
    if bg != Color32::TRANSPARENT {
        p.rect_filled(rect, CornerRadius::same(7), bg);
    }
    if resp.has_focus() {
        p.rect_stroke(
            rect,
            CornerRadius::same(7),
            Stroke::new(2.0_f32, theme.text),
            egui::StrokeKind::Inside,
        );
    }
    if let Some(tex) = icons.texture(ui.ctx(), kind, color, icon_px) {
        p.image(
            tex.id(),
            Rect::from_center_size(rect.center(), Vec2::splat(icon_px as f32)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }
    resp.on_hover_text(tooltip).clicked()
}
