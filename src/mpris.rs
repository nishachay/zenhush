//! MPRIS2 D-Bus media player interface (`org.mpris.MediaPlayer2.zenhush`).
//!
//! Enables hardware Play/Pause/Next/Prev media keys, `playerctl`, and
//! GNOME/KDE top-bar media widgets to view and control Zenhush natively.

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::HashMap;
    use std::sync::mpsc::Sender;
    use std::sync::{Arc, Mutex};
    use std::thread::Thread;

    use zbus::blocking::Connection;
    use zbus::interface;
    use zbus::object_server::SignalEmitter;
    use zbus::zvariant::{ObjectPath, Value};

    #[derive(Clone, Debug, PartialEq)]
    pub enum MprisCmd {
        Raise,
        Quit,
        Play,
        Pause,
        PlayPause,
        Stop,
        Next,
        Previous,
        SetVolume(u8),
    }

    #[derive(Clone, Debug, PartialEq)]
    pub struct MprisState {
        pub playing: bool,
        pub has_played: bool,
        pub title: String,
        pub subtitle: String,
        pub volume: u8,
        pub muted: bool,
    }

    impl Default for MprisState {
        fn default() -> Self {
            MprisState {
                playing: false,
                has_played: false,
                title: "Brown Noise".to_string(),
                subtitle: "deep and warm".to_string(),
                volume: 70,
                muted: false,
            }
        }
    }

    struct RootIface {
        tx: Sender<MprisCmd>,
        wake: Option<Arc<Thread>>,
    }

    impl RootIface {
        fn send(&self, cmd: MprisCmd) {
            let _ = self.tx.send(cmd);
            if let Some(ref w) = self.wake {
                w.unpark();
            }
        }
    }

    #[interface(name = "org.mpris.MediaPlayer2")]
    impl RootIface {
        fn raise(&self) {
            self.send(MprisCmd::Raise);
        }

        fn quit(&self) {
            self.send(MprisCmd::Quit);
        }

        #[zbus(property)]
        fn can_quit(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_raise(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn has_track_list(&self) -> bool {
            false
        }

        #[zbus(property)]
        fn identity(&self) -> &'static str {
            "Zenhush"
        }

        #[zbus(property)]
        fn desktop_entry(&self) -> &'static str {
            "zenhush"
        }

        #[zbus(property)]
        fn supported_uri_schemes(&self) -> Vec<&'static str> {
            vec!["file"]
        }

        #[zbus(property)]
        fn supported_mime_types(&self) -> Vec<&'static str> {
            vec![
                "audio/mpeg",
                "audio/flac",
                "audio/ogg",
                "audio/wav",
                "audio/mp4",
            ]
        }
    }

    struct PlayerIface {
        tx: Sender<MprisCmd>,
        wake: Option<Arc<Thread>>,
        state: Arc<Mutex<MprisState>>,
    }

    impl PlayerIface {
        fn send(&self, cmd: MprisCmd) {
            let _ = self.tx.send(cmd);
            if let Some(ref w) = self.wake {
                w.unpark();
            }
        }
        fn build_metadata(st: &MprisState) -> HashMap<String, Value<'static>> {
            let mut m = HashMap::new();
            if let Ok(path) = ObjectPath::try_from("/org/mpris/MediaPlayer2/Track/current") {
                m.insert("mpris:trackid".to_string(), Value::from(path));
            }
            m.insert("xesam:title".to_string(), Value::from(st.title.clone()));
            m.insert(
                "xesam:artist".to_string(),
                Value::from(vec!["Zenhush".to_string()]),
            );
            m.insert(
                "xesam:album".to_string(),
                Value::from(st.subtitle.clone()),
            );
            m
        }

        fn playback_status_str(st: &MprisState) -> &'static str {
            if st.playing {
                "Playing"
            } else if st.has_played {
                "Paused"
            } else {
                "Stopped"
            }
        }

        fn volume_f64(st: &MprisState) -> f64 {
            if st.muted {
                0.0
            } else {
                st.volume as f64 / 100.0
            }
        }
    }

    #[interface(name = "org.mpris.MediaPlayer2.Player")]
    impl PlayerIface {
        fn next(&self) {
            self.send(MprisCmd::Next);
        }

        fn previous(&self) {
            self.send(MprisCmd::Previous);
        }

        fn pause(&self) {
            self.send(MprisCmd::Pause);
        }

        fn play_pause(&self) {
            self.send(MprisCmd::PlayPause);
        }

        fn stop(&self) {
            self.send(MprisCmd::Stop);
        }

        fn play(&self) {
            self.send(MprisCmd::Play);
        }

        #[zbus(property)]
        fn playback_status(&self) -> &'static str {
            let st = self.state.lock().map(|g| g.clone()).unwrap_or_default();
            Self::playback_status_str(&st)
        }

        #[zbus(property)]
        fn metadata(&self) -> HashMap<String, Value<'static>> {
            let st = self.state.lock().map(|g| g.clone()).unwrap_or_default();
            Self::build_metadata(&st)
        }

        #[zbus(property)]
        fn volume(&self) -> f64 {
            let st = self.state.lock().map(|g| g.clone()).unwrap_or_default();
            Self::volume_f64(&st)
        }

        #[zbus(property)]
        fn set_volume(&self, v: f64) {
            let pct = (v.clamp(0.0, 1.0) * 100.0).round() as u8;
            self.send(MprisCmd::SetVolume(pct));
        }

        #[zbus(property)]
        fn rate(&self) -> f64 {
            1.0
        }

        #[zbus(property)]
        fn minimum_rate(&self) -> f64 {
            1.0
        }

        #[zbus(property)]
        fn maximum_rate(&self) -> f64 {
            1.0
        }

        #[zbus(property)]
        fn can_go_next(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_go_previous(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_play(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_pause(&self) -> bool {
            true
        }

        #[zbus(property)]
        fn can_seek(&self) -> bool {
            false
        }

        #[zbus(property)]
        fn can_control(&self) -> bool {
            true
        }
    }

    pub struct MprisServer {
        conn: Connection,
        state: Arc<Mutex<MprisState>>,
        last: Mutex<MprisState>,
    }

    impl MprisServer {
        pub fn start(tx: Sender<MprisCmd>, wake: Option<Arc<Thread>>) -> Option<Self> {
            let state = Arc::new(Mutex::new(MprisState::default()));
            let builder = match zbus::blocking::connection::Builder::session() {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("zenhush mpris session error: {}", e);
                    return None;
                }
            };
            let builder = match builder.name("org.mpris.MediaPlayer2.zenhush") {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("zenhush mpris name error: {}", e);
                    return None;
                }
            };
            let builder = match builder.serve_at(
                "/org/mpris/MediaPlayer2",
                RootIface {
                    tx: tx.clone(),
                    wake: wake.clone(),
                },
            ) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("zenhush mpris root serve_at error: {}", e);
                    return None;
                }
            };
            let builder = match builder.serve_at(
                "/org/mpris/MediaPlayer2",
                PlayerIface {
                    tx,
                    wake,
                    state: state.clone(),
                },
            ) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("zenhush mpris player serve_at error: {}", e);
                    return None;
                }
            };
            let conn = match builder.build() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("zenhush mpris build error: {}", e);
                    return None;
                }
            };

            Some(MprisServer {
                conn,
                state,
                last: Mutex::new(MprisState::default()),
            })
        }

        pub fn sync(&self, next: &MprisState) {
            let Ok(mut prev) = self.last.lock() else {
                return;
            };
            if *prev == *next {
                return;
            }
            let status_changed =
                prev.playing != next.playing || prev.has_played != next.has_played;
            let meta_changed = prev.title != next.title || prev.subtitle != next.subtitle;
            let vol_changed = prev.volume != next.volume || prev.muted != next.muted;
            *prev = next.clone();

            if let Ok(mut g) = self.state.lock() {
                *g = next.clone();
            }

            if let Ok(iface_ref) = self
                .conn
                .object_server()
                .interface::<_, PlayerIface>("/org/mpris/MediaPlayer2")
            {
                let emitter = SignalEmitter::from(iface_ref.signal_emitter().clone());
                let player = iface_ref.get();
                let _ = zbus::block_on(async {
                    if status_changed {
                        let _ = player.playback_status_changed(&emitter).await;
                    }
                    if meta_changed {
                        let _ = player.metadata_changed(&emitter).await;
                    }
                    if vol_changed {
                        let _ = player.volume_changed(&emitter).await;
                    }
                });
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod fallback {
    use std::sync::mpsc::Sender;

    #[derive(Clone, Debug, PartialEq)]
    pub enum MprisCmd {
        Raise,
        Quit,
        Play,
        Pause,
        PlayPause,
        Stop,
        Next,
        Previous,
        SetVolume(u8),
    }

    #[derive(Clone, Debug, PartialEq, Default)]
    pub struct MprisState {
        pub playing: bool,
        pub has_played: bool,
        pub title: String,
        pub subtitle: String,
        pub volume: u8,
        pub muted: bool,
    }

    pub struct MprisServer;
    impl MprisServer {
        pub fn start(_tx: Sender<MprisCmd>, _wake: Option<Arc<std::thread::Thread>>) -> Option<Self> {
            None
        }
        pub fn sync(&self, _next: &MprisState) {}
    }
}

#[cfg(target_os = "linux")]
pub use linux::{MprisCmd, MprisServer, MprisState};
#[cfg(not(target_os = "linux"))]
pub use fallback::{MprisCmd, MprisServer, MprisState};
