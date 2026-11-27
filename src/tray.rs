//! Cross-platform system tray / menu-bar service for Zenhush.
//!
//! - Linux: pure-Rust D-Bus StatusNotifierItem (`ksni`) — zero GTK dependency.
//!   Embeds `assets/tray.png` directly into the binary and switches to an
//!   amber-accented active icon whenever audio is playing.
//! - macOS / Windows: native `tray-icon` menu-bar / notification-area icon.

use std::sync::mpsc::{channel, Receiver};
#[cfg(target_os = "linux")]
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use crate::synth::Sound;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    ToggleWindow,
    TogglePlay,
    NextSound,
    PrevSound,
    SelectSound(Sound),
    SelectFile,
    PickFile,
    ToggleLockDefault,
    SetVolume(u8),
    VolumeStep(i8),
    ToggleMute,
    SetTimer(u32),
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrayState {
    pub playing: bool,
    pub sound_title: String,
    pub is_file: bool,
    pub has_file: bool,
    pub file_title: String,
    pub is_locked_default: bool,
    pub volume: u8,
    pub muted: bool,
    pub timer_minutes: u32,
    pub timer_label: String,
    pub notch_mode: bool,
    pub pinned: bool,
    pub visible: bool,
    pub tooltip: String,
}

impl Default for TrayState {
    fn default() -> Self {
        TrayState {
            playing: false,
            sound_title: "Brown Noise".to_string(),
            is_file: false,
            has_file: false,
            file_title: String::new(),
            is_locked_default: false,
            volume: 70,
            muted: false,
            timer_minutes: 0,
            timer_label: String::new(),
            notch_mode: false,
            pinned: false,
            visible: true,
            tooltip: "Zenhush — Ready".to_string(),
        }
    }
}

/// Build 32x32 RGBA pixels from embedded `assets/tray.png`.
/// When `active_amber` is true, renders a crisp amber dot badge in the corner
/// so the top bar reflects active playback at a glance.
fn build_rgba_icon(active_amber: bool) -> (u32, u32, Vec<u8>) {
    let raw = include_bytes!("../assets/tray.png");
    let mut img = match image::load_from_memory(raw) {
        Ok(decoded) => image::imageops::resize(
            &decoded.to_rgba8(),
            32,
            32,
            image::imageops::FilterType::Lanczos3,
        ),
        Err(_) => image::RgbaImage::new(32, 32),
    };

    if active_amber {
        // Warm amber dot at bottom-right (cx=25, cy=25, r=5)
        let cx = 25.0f32;
        let cy = 25.0f32;
        for y in 18..32u32 {
            for x in 18..32u32 {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist <= 5.2 {
                    // Dark ring around dot for contrast on light/dark bars
                    if dist >= 3.8 {
                        img.put_pixel(x, y, image::Rgba([16, 14, 12, 230]));
                    } else {
                        img.put_pixel(x, y, image::Rgba([200, 130, 62, 255]));
                    }
                }
            }
        }
    }

    let (w, h) = img.dimensions();
    (w, h, img.into_raw())
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use ksni::blocking::TrayMethods;
    use ksni::menu::{CheckmarkItem, MenuItem, StandardItem, SubMenu};
    use ksni::{Icon, ToolTip, Tray as KsniTray};

    fn to_ksni_icon(active_amber: bool) -> Vec<Icon> {
        let (w, h, rgba) = build_rgba_icon(active_amber);
        let mut argb = Vec::with_capacity(rgba.len());
        for px in rgba.chunks_exact(4) {
            argb.push(px[3]); // A
            argb.push(px[0]); // R
            argb.push(px[1]); // G
            argb.push(px[2]); // B
        }
        vec![Icon {
            width: w as i32,
            height: h as i32,
            data: argb,
        }]
    }

    struct ZenhushTray {
        tx: Sender<TrayEvent>,
        wake: Option<Arc<std::thread::Thread>>,
        idle_icon: Vec<Icon>,
        active_icon: Vec<Icon>,
        state: TrayState,
    }

    impl ZenhushTray {
        fn emit(&self, ev: TrayEvent) {
            let _ = self.tx.send(ev);
            if let Some(ref w) = self.wake {
                w.unpark();
            }
        }
    }

    impl KsniTray for ZenhushTray {
        fn id(&self) -> String {
            "zenhush".to_string()
        }

        fn title(&self) -> String {
            if self.state.playing {
                if !self.state.timer_label.is_empty() {
                    format!("Zenhush • {} ({})", self.state.sound_title, self.state.timer_label)
                } else {
                    format!("Zenhush • {}", self.state.sound_title)
                }
            } else {
                "Zenhush".to_string()
            }
        }

        fn icon_pixmap(&self) -> Vec<Icon> {
            if self.state.playing {
                self.active_icon.clone()
            } else {
                self.idle_icon.clone()
            }
        }

        fn tool_tip(&self) -> ToolTip {
            ToolTip {
                icon_name: String::new(),
                icon_pixmap: Vec::new(),
                title: "Zenhush".to_string(),
                description: self.state.tooltip.clone(),
            }
        }

        fn activate(&mut self, _x: i32, _y: i32) {
            self.emit(TrayEvent::ToggleWindow);
        }

        fn secondary_activate(&mut self, _x: i32, _y: i32) {
            self.emit(TrayEvent::TogglePlay);
        }

        fn scroll(&mut self, delta: i32, _orientation: ksni::Orientation) {
            if delta > 0 {
                self.emit(TrayEvent::VolumeStep(5));
            } else if delta < 0 {
                self.emit(TrayEvent::VolumeStep(-5));
            }
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            let st = &self.state;
            let play_label = if st.playing {
                format!("⏸  Pause — {}", st.sound_title)
            } else {
                format!("▶  Play — {}", st.sound_title)
            };

            let mut items: Vec<MenuItem<Self>> = vec![
                StandardItem {
                    label: play_label,
                    activate: Box::new(move |this: &mut Self| {
                        this.emit(TrayEvent::TogglePlay);
                    }),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "Next Sound".into(),
                    activate: Box::new(move |this: &mut Self| {
                        this.emit(TrayEvent::NextSound);
                    }),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: "Previous Sound".into(),
                    activate: Box::new(move |this: &mut Self| {
                        this.emit(TrayEvent::PrevSound);
                    }),
                    ..Default::default()
                }
                .into(),
                MenuItem::Separator,
            ];

            // Sounds submenu
            let mut sound_items: Vec<MenuItem<Self>> = Sound::ALL
                .iter()
                .map(|&sound| {
                    let checked = !st.is_file && st.sound_title == sound.title();
                    CheckmarkItem {
                        label: sound.title().into(),
                        checked,
                        activate: Box::new(move |this: &mut Self| {
                            this.emit(TrayEvent::SelectSound(sound));
                        }),
                        ..Default::default()
                    }
                    .into()
                })
                .collect();

            if st.has_file {
                sound_items.push(MenuItem::Separator);
                let prefix = if st.is_locked_default { "🔒 " } else { "" };
                sound_items.push(
                    CheckmarkItem {
                        label: format!("{}File: {}", prefix, st.file_title),
                        checked: st.is_file,
                        activate: Box::new(move |this: &mut Self| {
                            this.emit(TrayEvent::SelectFile);
                        }),
                        ..Default::default()
                    }
                    .into(),
                );
                sound_items.push(
                    CheckmarkItem {
                        label: "Lock Local File as Default".into(),
                        checked: st.is_locked_default,
                        activate: Box::new(move |this: &mut Self| {
                            this.emit(TrayEvent::ToggleLockDefault);
                        }),
                        ..Default::default()
                    }
                    .into(),
                );
            }
            sound_items.push(MenuItem::Separator);
            sound_items.push(
                StandardItem {
                    label: "Choose Audio File…".into(),
                    activate: Box::new(move |this: &mut Self| {
                        this.emit(TrayEvent::PickFile);
                    }),
                    ..Default::default()
                }
                .into(),
            );

            items.push(
                SubMenu {
                    label: "Sound".into(),
                    submenu: sound_items,
                    ..Default::default()
                }
                .into(),
            );

            // Volume submenu
            let mut vol_items: Vec<MenuItem<Self>> = vec![CheckmarkItem {
                label: "Mute".into(),
                checked: st.muted,
                activate: Box::new(move |this: &mut Self| {
                    this.emit(TrayEvent::ToggleMute);
                }),
                ..Default::default()
            }
            .into()];
            for v in [100u8, 80, 70, 50, 30, 15] {
                vol_items.push(
                    CheckmarkItem {
                        label: format!("{}%", v),
                        checked: !st.muted && st.volume == v,
                        activate: Box::new(move |this: &mut Self| {
                            this.emit(TrayEvent::SetVolume(v));
                        }),
                        ..Default::default()
                    }
                    .into(),
                );
            }
            items.push(
                SubMenu {
                    label: if st.muted {
                        "Volume (Muted)".into()
                    } else {
                        format!("Volume ({}%)", st.volume)
                    },
                    submenu: vol_items,
                    ..Default::default()
                }
                .into(),
            );

            // Sleep Timer submenu
            let timer_opts = [
                (0u32, "Off"),
                (15, "15 min"),
                (25, "25 min (Focus)"),
                (30, "30 min"),
                (45, "45 min"),
                (60, "60 min"),
                (90, "90 min"),
            ];
            let timer_items: Vec<MenuItem<Self>> = timer_opts
                .iter()
                .map(|&(m, lbl)| {
                    CheckmarkItem {
                        label: lbl.into(),
                        checked: st.timer_minutes == m,
                        activate: Box::new(move |this: &mut Self| {
                            this.emit(TrayEvent::SetTimer(m));
                        }),
                        ..Default::default()
                    }
                    .into()
                })
                .collect();
            items.push(
                SubMenu {
                    label: if st.timer_label.is_empty() {
                        "Timer".into()
                    } else {
                        format!("Timer ({})", st.timer_label)
                    },
                    submenu: timer_items,
                    ..Default::default()
                }
                .into(),
            );

            items.push(MenuItem::Separator);

            items.push(
                StandardItem {
                    label: if st.visible {
                        "Hide Controls".into()
                    } else {
                        "Open Controls".into()
                    },
                    activate: Box::new(move |this: &mut Self| {
                        this.emit(TrayEvent::ToggleWindow);
                    }),
                    ..Default::default()
                }
                .into(),
            );

            items.push(
                StandardItem {
                    label: "Quit Zenhush".into(),
                    activate: Box::new(move |this: &mut Self| {
                        this.emit(TrayEvent::Quit);
                    }),
                    ..Default::default()
                }
                .into(),
            );

            items
        }
    }

    pub struct Tray {
        pub rx: Receiver<TrayEvent>,
        last_state: Arc<Mutex<TrayState>>,
        handle: ksni::blocking::Handle<ZenhushTray>,
    }

    impl Tray {
        pub fn new(wake: Option<Arc<std::thread::Thread>>) -> Option<Self> {
            let (tx, rx) = channel::<TrayEvent>();
            let state = TrayState::default();
            let tray = ZenhushTray {
                tx,
                wake,
                idle_icon: to_ksni_icon(false),
                active_icon: to_ksni_icon(true),
                state: state.clone(),
            };
            let handle = tray.spawn().ok()?;
            Some(Tray {
                rx,
                last_state: Arc::new(Mutex::new(state)),
                handle,
            })
        }

        pub fn sync_state(&self, next: &TrayState) {
            let Ok(mut prev) = self.last_state.lock() else {
                return;
            };
            if *prev == *next {
                return;
            }
            *prev = next.clone();
            let cloned = next.clone();
            self.handle.update(move |tray| {
                tray.state = cloned;
            });
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    use super::*;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    fn to_tray_icon(active_amber: bool) -> Option<tray_icon::Icon> {
        let (w, h, rgba) = build_rgba_icon(active_amber);
        tray_icon::Icon::from_rgba(rgba, w, h).ok()
    }

    pub struct Tray {
        pub rx: Receiver<TrayEvent>,
        last_state: Arc<Mutex<TrayState>>,
        tray: TrayIcon,
        idle_icon: Option<tray_icon::Icon>,
        active_icon: Option<tray_icon::Icon>,
    }

    impl Tray {
        pub fn new(wake: Option<Arc<std::thread::Thread>>) -> Option<Self> {
            let (tx, rx) = channel::<TrayEvent>();
            let idle_icon = to_tray_icon(false);
            let active_icon = to_tray_icon(true);

            let menu = Menu::new();
            let item_show = MenuItem::with_id("show", "Open / Hide Zenhush", true, None);
            let item_play = MenuItem::with_id("play", "Play / Pause", true, None);
            let item_next = MenuItem::with_id("next", "Next Sound", true, None);
            let item_pick = MenuItem::with_id("pick", "Choose Audio File…", true, None);
            let item_quit = MenuItem::with_id("quit", "Quit Zenhush", true, None);
            let _ = menu.append_items(&[
                &item_play,
                &item_next,
                &item_pick,
                &PredefinedMenuItem::separator(),
                &item_show,
                &PredefinedMenuItem::separator(),
                &item_quit,
            ]);

            let mut builder = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_tooltip("Zenhush — Focus Audio");
            if let Some(icon) = idle_icon.clone() {
                builder = builder.with_icon(icon);
            }
            let tray = builder.build().ok()?;

            let tx_click = tx.clone();
            let wake_click = wake.clone();
            std::thread::spawn(move || {
                let receiver = TrayIconEvent::receiver();
                while let Ok(ev) = receiver.recv() {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = ev
                    {
                        let _ = tx_click.send(TrayEvent::ToggleWindow);
                        if let Some(ref w) = wake_click {
                            w.unpark();
                        }
                    }
                }
            });

            let tx_menu = tx;
            std::thread::spawn(move || {
                let receiver = MenuEvent::receiver();
                while let Ok(ev) = receiver.recv() {
                    let mapped = match ev.id().0.as_str() {
                        "show" => Some(TrayEvent::ToggleWindow),
                        "play" => Some(TrayEvent::TogglePlay),
                        "next" => Some(TrayEvent::NextSound),
                        "pick" => Some(TrayEvent::PickFile),
                        "quit" => Some(TrayEvent::Quit),
                        _ => None,
                    };
                    if let Some(ev) = mapped {
                        let _ = tx_menu.send(ev);
                        if let Some(ref w) = wake {
                            w.unpark();
                        }
                    }
                }
            });

            Some(Tray {
                rx,
                last_state: Arc::new(Mutex::new(TrayState::default())),
                tray,
                idle_icon,
                active_icon,
            })
        }

        pub fn sync_state(&self, next: &TrayState) {
            let Ok(mut prev) = self.last_state.lock() else {
                return;
            };
            if *prev == *next {
                return;
            }
            let playing_changed = prev.playing != next.playing;
            *prev = next.clone();
            let _ = self.tray.set_tooltip(Some(&next.tooltip));
            if playing_changed {
                let icon = if next.playing {
                    self.active_icon.clone()
                } else {
                    self.idle_icon.clone()
                };
                if let Some(i) = icon {
                    let _ = self.tray.set_icon(Some(i));
                }
            }
        }
    }
}

pub use platform::Tray;
