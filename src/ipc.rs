//! Single-instance IPC and CLI command parsing for Zenhush.
//!
//! Enables instant background wakeup via `zenhush --toggle` (or any system
//! global shortcut bound to `zenhush --toggle`), media control CLI flags,
//! and `--background` startup in the top bar / menu bar.

use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::thread::{self, Thread};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteCmd {
    Toggle,
    Show,
    PlayPause,
    Play,
    Pause,
    Next,
    Prev,
    VolUp,
    VolDown,
    Mute,
    ToggleNotch,
    Quit,
}

impl RemoteCmd {
    pub fn as_wire(self) -> &'static str {
        match self {
            RemoteCmd::Toggle => "toggle",
            RemoteCmd::Show => "show",
            RemoteCmd::PlayPause => "play-pause",
            RemoteCmd::Play => "play",
            RemoteCmd::Pause => "pause",
            RemoteCmd::Next => "next",
            RemoteCmd::Prev => "prev",
            RemoteCmd::VolUp => "vol-up",
            RemoteCmd::VolDown => "vol-down",
            RemoteCmd::Mute => "mute",
            RemoteCmd::ToggleNotch => "notch",
            RemoteCmd::Quit => "quit",
        }
    }

    pub fn from_wire(s: &str) -> Option<Self> {
        match s.trim() {
            "toggle" => Some(RemoteCmd::Toggle),
            "show" => Some(RemoteCmd::Show),
            "play-pause" => Some(RemoteCmd::PlayPause),
            "play" => Some(RemoteCmd::Play),
            "pause" => Some(RemoteCmd::Pause),
            "next" => Some(RemoteCmd::Next),
            "prev" => Some(RemoteCmd::Prev),
            "vol-up" => Some(RemoteCmd::VolUp),
            "vol-down" => Some(RemoteCmd::VolDown),
            "mute" => Some(RemoteCmd::Mute),
            "notch" => Some(RemoteCmd::ToggleNotch),
            "quit" => Some(RemoteCmd::Quit),
            _ => None,
        }
    }
}

pub struct CliArgs {
    pub start_background: bool,
    pub ui_mode: bool,
    pub daemon_mode: bool,
    pub command: Option<RemoteCmd>,
    pub help_requested: bool,
    pub install_requested: bool,
    pub uninstall_requested: bool,
    pub install_gnome_pill: bool,
    pub uninstall_gnome_pill: bool,
}

pub fn parse_cli() -> CliArgs {
    let mut start_background = false;
    let mut ui_mode = false;
    let mut daemon_mode = false;
    let mut command = None;
    let mut help_requested = false;
    let mut install_requested = false;
    let mut uninstall_requested = false;
    let mut install_gnome_pill = false;
    let mut uninstall_gnome_pill = false;

    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--background" | "--minimized" | "-b" => start_background = true,
            "--ui" => ui_mode = true,
            "--daemon" => daemon_mode = true,
            "--toggle" | "-t" => command = Some(RemoteCmd::Toggle),
            "--show" => command = Some(RemoteCmd::Show),
            "--play-pause" | "-p" => command = Some(RemoteCmd::PlayPause),
            "--play" => command = Some(RemoteCmd::Play),
            "--pause" => command = Some(RemoteCmd::Pause),
            "--next" => command = Some(RemoteCmd::Next),
            "--prev" => command = Some(RemoteCmd::Prev),
            "--vol-up" => command = Some(RemoteCmd::VolUp),
            "--vol-down" => command = Some(RemoteCmd::VolDown),
            "--mute" => command = Some(RemoteCmd::Mute),
            "--notch" => command = Some(RemoteCmd::ToggleNotch),
            "--quit" => command = Some(RemoteCmd::Quit),
            "--install" => install_requested = true,
            "--uninstall" => uninstall_requested = true,
            "--install-gnome-pill" => install_gnome_pill = true,
            "--uninstall-gnome-pill" => uninstall_gnome_pill = true,
            "--help" | "-h" => help_requested = true,
            _ => {}
        }
    }

    CliArgs {
        start_background,
        ui_mode,
        daemon_mode,
        command,
        help_requested,
        install_requested,
        uninstall_requested,
        install_gnome_pill,
        uninstall_gnome_pill,
    }
}

pub fn print_help() {
    println!(
        "Zenhush — lightweight focus audio machine\n\n\
        Usage: zenhush [OPTIONS]\n\n\
        Options:\n\
          -b, --background           Start silently in the top bar / tray (~15 MB RAM)\n\
          -t, --toggle               Toggle popup visibility (wakes running instance)\n\
              --show                 Show and focus Zenhush\n\
          -p, --play-pause           Toggle playback\n\
              --play                 Start / resume playback\n\
              --pause                Pause playback\n\
              --next                 Next sound\n\
              --prev                 Previous sound\n\
              --vol-up               Increase volume (+5%)\n\
              --vol-down             Decrease volume (-5%)\n\
              --mute                 Toggle mute\n\
              --notch                Toggle compact Notch Pill mode\n\
              --quit                 Quit running instance\n\
              --install              Install binary, app launcher & icon to ~/.local\n\
              --uninstall            Remove installed ~/.local binary, launcher & autostart\n\
              --install-gnome-pill   Install optional GNOME Top-Bar Pill extension\n\
              --uninstall-gnome-pill Remove GNOME Top-Bar Pill extension\n\
          -h, --help                 Show this help"
    );
}

#[cfg(unix)]
fn socket_path() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        std::path::PathBuf::from(dir).join("zenhush.sock")
    } else {
        std::env::temp_dir().join(format!(
            "zenhush-{}.sock",
            std::env::var("USER").unwrap_or_else(|_| "user".into())
        ))
    }
}

#[cfg(not(unix))]
fn port_file_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("zenhush")
        .join("ipc.port")
}

/// Try sending a command to an already-running Zenhush instance.
/// Returns `true` if another instance received the command.
pub fn try_send_to_existing(cmd: RemoteCmd) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixStream;
        let path = socket_path();
        if let Ok(mut stream) = UnixStream::connect(&path) {
            let _ = writeln!(stream, "{}", cmd.as_wire());
            let _ = stream.flush();
            return true;
        }
        false
    }
    #[cfg(not(unix))]
    {
        use std::net::TcpStream;
        let Ok(port_str) = std::fs::read_to_string(port_file_path()) else {
            return false;
        };
        let Ok(port) = port_str.trim().parse::<u16>() else {
            return false;
        };
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
            let _ = writeln!(stream, "{}", cmd.as_wire());
            let _ = stream.flush();
            return true;
        }
        false
    }
}

pub struct IpcServer {
    pub rx: Receiver<RemoteCmd>,
    #[cfg(unix)]
    sock_path: std::path::PathBuf,
}

impl IpcServer {
    pub fn start(wake: Option<Arc<Thread>>) -> Option<Self> {
        let (tx, rx) = channel::<RemoteCmd>();

        #[cfg(unix)]
        {
            use std::os::unix::net::UnixListener;
            let path = socket_path();
            if path.exists() {
                let _ = std::fs::remove_file(&path);
            }
            let listener = UnixListener::bind(&path).ok()?;
            let _ = thread::Builder::new()
                .name("zenhush-ipc".into())
                .spawn(move || {
                    for stream in listener.incoming().flatten() {
                        let mut reader = BufReader::new(stream);
                        let mut line = String::new();
                        if reader.read_line(&mut line).is_ok() {
                            if let Some(cmd) = RemoteCmd::from_wire(&line) {
                                let _ = tx.send(cmd);
                                if let Some(ref t) = wake {
                                    t.unpark();
                                }
                            }
                        }
                    }
                });
            Some(IpcServer {
                rx,
                sock_path: path,
            })
        }

        #[cfg(not(unix))]
        {
            use std::net::TcpListener;
            let listener = TcpListener::bind(("127.0.0.1", 0)).ok()?;
            let port = listener.local_addr().ok()?.port();
            let pfile = port_file_path();
            if let Some(parent) = pfile.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&pfile, port.to_string());
            let _ = thread::Builder::new()
                .name("zenhush-ipc".into())
                .spawn(move || {
                    for stream in listener.incoming().flatten() {
                        let mut reader = BufReader::new(stream);
                        let mut line = String::new();
                        if reader.read_line(&mut line).is_ok() {
                            if let Some(cmd) = RemoteCmd::from_wire(&line) {
                                let _ = tx.send(cmd);
                                if let Some(ref t) = wake {
                                    t.unpark();
                                }
                            }
                        }
                    }
                });
            Some(IpcServer { rx })
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&self.sock_path);
        }
        #[cfg(not(unix))]
        {
            let _ = std::fs::remove_file(port_file_path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_cmd_wire_roundtrip() {
        for cmd in [
            RemoteCmd::Toggle,
            RemoteCmd::Show,
            RemoteCmd::PlayPause,
            RemoteCmd::Play,
            RemoteCmd::Pause,
            RemoteCmd::Next,
            RemoteCmd::Prev,
            RemoteCmd::VolUp,
            RemoteCmd::VolDown,
            RemoteCmd::Mute,
            RemoteCmd::ToggleNotch,
            RemoteCmd::Quit,
        ] {
            assert_eq!(RemoteCmd::from_wire(cmd.as_wire()), Some(cmd));
        }
    }
}
