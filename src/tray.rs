//! System tray icon + menu: "Show Zenhush" and "Quit".
//! Left-click toggles the popup; the menu offers the same actions.
//! The tray runs on its own thread (ksni, pure Rust D-Bus, no GTK)
//! and sends events over a channel the app polls each frame.
//! When no tray host is available (headless VM, some desktops) the
//! app still runs and the Quit button lives in the settings panel.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use image::ImageReader;
use ksni::blocking::TrayMethods;
use ksni::menu::{MenuItem, StandardItem};
use ksni::{Icon, ToolTip, Tray as KsniTray};

/// Events the tray sends to the app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// Left-click on the icon: toggle the popup.
    Toggle,
    /// Menu "Show Zenhush": show the popup.
    Show,
    /// Menu "Quit": quit the app.
    Quit,
}

/// The tray service. Lives on ksni's thread; talks to the app through
/// a channel (events) and a shared tooltip string.
pub struct ZenhushTray {
    tx: Sender<TrayEvent>,
    icon: Vec<Icon>,
    tooltip: Arc<Mutex<String>>,
}

impl ZenhushTray {
    fn load_icon() -> Vec<Icon> {
        // Load the tray PNG and shrink it to 32x32. ksni wants ARGB32
        // in network byte order: each pixel is [A, R, G, B].
        let img = ImageReader::open("assets/tray.png")
            .ok()
            .and_then(|r| r.decode().ok())
            .map(|i| i.to_rgba8());
        let Some(img) = img else {
            return Vec::new();
        };
        let small =
            image::imageops::resize(&img, 32, 32, image::imageops::FilterType::Lanczos3);
        let (w, h) = small.dimensions();
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        for px in small.pixels() {
            data.push(px[3]); // A
            data.push(px[0]); // R
            data.push(px[1]); // G
            data.push(px[2]); // B
        }
        vec![Icon {
            width: w as i32,
            height: h as i32,
            data,
        }]
    }
}

impl KsniTray for ZenhushTray {
    fn id(&self) -> String {
        "zenhush".to_string()
    }

    fn title(&self) -> String {
        "Zenhush".to_string()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        self.icon.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
            title: "Zenhush".to_string(),
            description: self
                .tooltip
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default(),
        }
    }

    /// Left-click: toggle the popup.
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send(TrayEvent::Toggle);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let tx_show = self.tx.clone();
        let tx_quit = self.tx.clone();
        vec![
            StandardItem {
                label: "Show Zenhush".into(),
                activate: Box::new(move |_| {
                    let _ = tx_show.send(TrayEvent::Show);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(move |_| {
                    let _ = tx_quit.send(TrayEvent::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// The tray handle the app holds: the event channel plus a way to
/// update the tooltip.
pub struct Tray {
    pub rx: Receiver<TrayEvent>,
    tooltip: Arc<Mutex<String>>,
    /// The ksni service handle (keeps the tray alive). Held, not used.
    #[allow(dead_code)]
    handle: ksni::blocking::Handle<ZenhushTray>,
}

impl Tray {
    /// Start the tray service. Returns None when no tray host is
    /// available (the app still runs; Quit lives in the settings panel).
    pub fn new() -> Option<Self> {
        let (tx, rx) = channel::<TrayEvent>();
        let tooltip = Arc::new(Mutex::new("Zenhush".to_string()));
        let tray = ZenhushTray {
            tx,
            icon: ZenhushTray::load_icon(),
            tooltip: tooltip.clone(),
        };
        let handle = tray.spawn().ok()?;
        Some(Tray {
            rx,
            tooltip,
            handle,
        })
    }

    /// Update the tooltip to show what is playing (or why not).
    pub fn set_tooltip(&self, text: &str) {
        if let Ok(mut g) = self.tooltip.lock() {
            *g = text.to_string();
        }
    }
}
