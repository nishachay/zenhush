//! Lightweight JSON-lines bridge between the ~15 MB Zenhush background Core
//! (`--daemon` / `--background`) and the on-demand `--ui` window process.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;

use serde::{Deserialize, Serialize};

use crate::settings::{DockAnchor, LocalTrack, ThemeMode, WindowMode};
use crate::synth::{FileTone, Sound};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UiSnapshot {
    pub playing: bool,
    pub has_played: bool,
    pub is_file_mode: bool,
    pub active_sound: Sound,
    pub active_file: Option<PathBuf>,
    pub locked_default_file: Option<PathBuf>,
    pub local_tracks: Vec<LocalTrack>,
    pub file_tone: FileTone,
    pub loop_crossfade: bool,
    pub layer_sound: Option<Sound>,
    pub layer_mix: u8,
    pub volume: u8,
    pub muted: bool,
    pub depth: u8,
    pub rumble: u8,
    pub elapsed_secs: u64,
    pub timer_minutes: u32,
    pub timer_remaining_secs: Option<u64>,
    pub rms_bars: Vec<f32>,
    pub audio_error: Option<String>,
    pub device_name: String,
    pub notice: Option<(String, bool)>,
    pub missing_locked_file: Option<PathBuf>,

    pub theme_mode: ThemeMode,
    pub reduce_motion: bool,
    pub window_mode: WindowMode,
    pub dock_anchor: DockAnchor,
    pub saved_pos: Option<[f32; 2]>,
    pub pinned: bool,
    pub auto_play_on_launch: bool,
    pub start_in_background: bool,
    pub autostart_enabled: bool,
    pub gnome_pill_installed: bool,
    pub global_shortcut_toggle: String,
    pub global_shortcut_play: String,
    pub shortcut_status: String,
    pub has_tray: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CoreToUi {
    Snapshot(UiSnapshot),
    Focus,
    Close,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum UiCmd {
    TogglePlay,
    Restart,
    StepSound(i32),
    SelectSound(Sound),
    SelectFileMode,
    AddLocalFile {
        path: PathBuf,
        play_now: bool,
        lock_default: bool,
    },
    RemoveLocalTrack(PathBuf),
    ToggleLockDefault(PathBuf),
    UnlockMissingDefault,
    SetFileTone(FileTone),
    SetLoopCrossfade(bool),
    SetLayer(Option<Sound>),
    SetLayerMix(u8),
    SetVolume(u8),
    ToggleMute,
    SetDepth(u8),
    SetRumble(u8),
    ResetControls,
    SetTimer(u32),
    CycleTimer,
    SetTheme(ThemeMode),
    SetReduceMotion(bool),
    SetWindowMode(WindowMode),
    SetDockAnchor(DockAnchor),
    SetSavedPos(Option<[f32; 2]>),
    SetPinned(bool),
    SetAutoPlay(bool),
    SetStartInBackground(bool),
    SetAutostart(bool),
    InstallLauncher,
    ToggleGnomePill,
    BindGnomeShortcut,
    SetGlobalShortcutToggle(String),
    QuitDaemon,
}

#[cfg(unix)]
pub fn bridge_socket_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(dir).join("zenhush-ui.sock")
    } else {
        std::env::temp_dir().join(format!(
            "zenhush-ui-{}.sock",
            std::env::var("USER").unwrap_or_else(|_| "user".into())
        ))
    }
}

#[cfg(not(unix))]
fn bridge_port_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("zenhush")
        .join("ui.port")
}

pub fn bridge_ready() -> bool {
    #[cfg(unix)]
    {
        std::os::unix::net::UnixStream::connect(bridge_socket_path()).is_ok()
    }
    #[cfg(not(unix))]
    {
        let Ok(p) = std::fs::read_to_string(bridge_port_path()) else {
            return false;
        };
        let Ok(port) = p.trim().parse::<u16>() else {
            return false;
        };
        std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
    }
}

/// Core-side server managing the currently connected `--ui` process (if any).
pub struct BridgeServer {
    pub cmd_rx: Receiver<UiCmd>,
    active_writer: Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    #[cfg(unix)]
    sock_path: PathBuf,
}

impl BridgeServer {
    pub fn start(wake: Option<Arc<std::thread::Thread>>) -> Option<Self> {
        let (cmd_tx, cmd_rx) = channel::<UiCmd>();
        let active_writer: Arc<Mutex<Option<Box<dyn Write + Send>>>> =
            Arc::new(Mutex::new(None));

        #[cfg(unix)]
        {
            use std::os::unix::net::UnixListener;
            let path = bridge_socket_path();
            if path.exists() {
                let _ = std::fs::remove_file(&path);
            }
            let listener = UnixListener::bind(&path).ok()?;
            let writer_slot = active_writer.clone();
            let _ = thread::Builder::new()
                .name("zenhush-ui-bridge".into())
                .spawn(move || {
                    for stream in listener.incoming().flatten() {
                        let Ok(reader_stream) = stream.try_clone() else {
                            continue;
                        };
                        if let Ok(mut g) = writer_slot.lock() {
                            *g = Some(Box::new(stream));
                        }
                        if let Some(ref w) = wake {
                            w.unpark();
                        }
                        let tx = cmd_tx.clone();
                        let slot_cleanup = writer_slot.clone();
                        let wake_r = wake.clone();
                        let _ = thread::Builder::new()
                            .name("zenhush-ui-reader".into())
                            .spawn(move || {
                                let reader = BufReader::new(reader_stream);
                                for line in reader.lines().map_while(Result::ok) {
                                    if let Ok(cmd) = serde_json::from_str::<UiCmd>(&line) {
                                        let _ = tx.send(cmd);
                                        if let Some(ref w) = wake_r {
                                            w.unpark();
                                        }
                                    }
                                }
                                if let Ok(mut g) = slot_cleanup.lock() {
                                    *g = None;
                                }
                                if let Some(ref w) = wake_r {
                                    w.unpark();
                                }
                            });
                    }
                });
            Some(BridgeServer {
                cmd_rx,
                active_writer,
                sock_path: path,
            })
        }

        #[cfg(not(unix))]
        {
            use std::net::TcpListener;
            let listener = TcpListener::bind(("127.0.0.1", 0)).ok()?;
            let port = listener.local_addr().ok()?.port();
            let pfile = bridge_port_path();
            if let Some(parent) = pfile.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&pfile, port.to_string());
            let writer_slot = active_writer.clone();
            let _ = thread::Builder::new()
                .name("zenhush-ui-bridge".into())
                .spawn(move || {
                    for stream in listener.incoming().flatten() {
                        let Ok(reader_stream) = stream.try_clone() else {
                            continue;
                        };
                        if let Ok(mut g) = writer_slot.lock() {
                            *g = Some(Box::new(stream));
                        }
                        if let Some(ref w) = wake {
                            w.unpark();
                        }
                        let tx = cmd_tx.clone();
                        let slot_cleanup = writer_slot.clone();
                        let wake_r = wake.clone();
                        let _ = thread::Builder::new()
                            .name("zenhush-ui-reader".into())
                            .spawn(move || {
                                let reader = BufReader::new(reader_stream);
                                for line in reader.lines().map_while(Result::ok) {
                                    if let Ok(cmd) = serde_json::from_str::<UiCmd>(&line) {
                                        let _ = tx.send(cmd);
                                        if let Some(ref w) = wake_r {
                                            w.unpark();
                                        }
                                    }
                                }
                                if let Ok(mut g) = slot_cleanup.lock() {
                                    *g = None;
                                }
                                if let Some(ref w) = wake_r {
                                    w.unpark();
                                }
                            });
                    }
                });
            Some(BridgeServer {
                cmd_rx,
                active_writer,
            })
        }
    }

    pub fn has_client(&self) -> bool {
        self.active_writer
            .lock()
            .map(|g| g.is_some())
            .unwrap_or(false)
    }

    pub fn send(&self, msg: &CoreToUi) {
        let Ok(line) = serde_json::to_string(msg) else {
            return;
        };
        let Ok(mut guard) = self.active_writer.lock() else {
            return;
        };
        if let Some(ref mut w) = *guard {
            if writeln!(w, "{}", line).is_err() || w.flush().is_err() {
                *guard = None;
            }
        }
    }
}

impl Drop for BridgeServer {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&self.sock_path);
        }
        #[cfg(not(unix))]
        {
            let _ = std::fs::remove_file(bridge_port_path());
        }
    }
}

/// UI-side client connecting to Core over `zenhush-ui.sock`.
pub struct BridgeClient {
    pub rx: Receiver<CoreToUi>,
    writer: Mutex<Box<dyn Write + Send>>,
    egui_ctx: Arc<Mutex<Option<eframe::egui::Context>>>,
}

impl BridgeClient {
    pub fn connect() -> Result<Self, String> {
        #[cfg(unix)]
        let stream = std::os::unix::net::UnixStream::connect(bridge_socket_path())
            .map_err(|e| format!("Could not connect to Zenhush background core ({})", e))?;
        #[cfg(not(unix))]
        let stream = {
            let p = std::fs::read_to_string(bridge_port_path())
                .map_err(|e| format!("No background core port ({})", e))?;
            let port: u16 = p.trim().parse().map_err(|_| "Invalid port")?;
            std::net::TcpStream::connect(("127.0.0.1", port))
                .map_err(|e| format!("Could not connect to background core ({})", e))?
        };

        let reader_stream = stream.try_clone().map_err(|e| e.to_string())?;
        let (tx, rx) = channel::<CoreToUi>();
        let egui_ctx: Arc<Mutex<Option<eframe::egui::Context>>> = Arc::new(Mutex::new(None));
        let wake_ctx = egui_ctx.clone();

        let _ = thread::Builder::new()
            .name("zenhush-ui-client".into())
            .spawn(move || {
                let reader = BufReader::new(reader_stream);
                for line in reader.lines().map_while(Result::ok) {
                    if let Ok(msg) = serde_json::from_str::<CoreToUi>(&line) {
                        let _ = tx.send(msg);
                        if let Ok(g) = wake_ctx.lock() {
                            if let Some(ctx) = &*g {
                                ctx.request_repaint();
                            }
                        }
                    }
                }
            });

        Ok(BridgeClient {
            rx,
            writer: Mutex::new(Box::new(stream)),
            egui_ctx,
        })
    }

    pub fn bind_egui_ctx(&self, ctx: &eframe::egui::Context) {
        if let Ok(mut g) = self.egui_ctx.lock() {
            *g = Some(ctx.clone());
        }
    }

    pub fn send(&self, cmd: &UiCmd) {
        let Ok(line) = serde_json::to_string(cmd) else {
            return;
        };
        if let Ok(mut w) = self.writer.lock() {
            let _ = writeln!(w, "{}", line);
            let _ = w.flush();
        }
    }
}
