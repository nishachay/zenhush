//! Cross-platform global shortcuts + Linux desktop/GNOME integration.
//!
//! - On X11, macOS, and Windows: registers live global hotkeys in-process via `global-hotkey`.
//! - On Wayland (GNOME): Wayland intentionally blocks passive X11 key grabs, so we
//!   also provide 1-click GNOME `gsettings` custom-keybinding registration targeting
//!   `zenhush --toggle` (and autostart + `.desktop` app launcher installation).

use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::thread::Thread;

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlobalAction {
    TogglePopup,
    TogglePlay,
}

pub struct ShortcutManager {
    manager: Option<GlobalHotKeyManager>,
    toggle_hotkey: Option<HotKey>,
    play_hotkey: Option<HotKey>,
    event_rx: Option<Receiver<GlobalHotKeyEvent>>,
    pub status_note: String,
}

impl ShortcutManager {
    pub fn new(toggle_str: &str, play_str: &str, wake: Option<Arc<Thread>>) -> Self {
        let manager = GlobalHotKeyManager::new().ok();
        let event_rx = if manager.is_some() {
            let (tx, rx) = channel::<GlobalHotKeyEvent>();
            let _ = std::thread::Builder::new()
                .name("zenhush-hotkey-wake".into())
                .spawn(move || {
                    let g_rx = GlobalHotKeyEvent::receiver();
                    while let Ok(ev) = g_rx.recv() {
                        let _ = tx.send(ev);
                        if let Some(ref w) = wake {
                            w.unpark();
                        }
                    }
                });
            Some(rx)
        } else {
            None
        };

        let mut s = ShortcutManager {
            manager,
            toggle_hotkey: None,
            play_hotkey: None,
            event_rx,
            status_note: String::new(),
        };
        s.apply(toggle_str, play_str);
        s
    }

    pub fn apply(&mut self, toggle_str: &str, play_str: &str) {
        let Some(mgr) = &self.manager else {
            self.status_note = if is_wayland() {
                "Wayland session: use System Shortcut below".to_string()
            } else {
                "Global hotkey manager unavailable".to_string()
            };
            return;
        };

        if let Some(old) = self.toggle_hotkey.take() {
            let _ = mgr.unregister(old);
        }
        if let Some(old) = self.play_hotkey.take() {
            let _ = mgr.unregister(old);
        }

        let mut notes = Vec::new();
        if let Ok(hk) = HotKey::from_str(toggle_str) {
            if mgr.register(hk).is_ok() {
                self.toggle_hotkey = Some(hk);
            } else {
                notes.push(format!("Could not bind {}", toggle_str));
            }
        }
        if let Ok(hk) = HotKey::from_str(play_str) {
            if mgr.register(hk).is_ok() {
                self.play_hotkey = Some(hk);
            } else {
                notes.push(format!("Could not bind {}", play_str));
            }
        }

        if notes.is_empty() {
            if is_wayland() {
                self.status_note =
                    "XWayland hotkeys active; bind GNOME shortcut for universal Wayland focus"
                        .to_string();
            } else {
                self.status_note = "Global shortcuts active".to_string();
            }
        } else {
            self.status_note = notes.join(" • ");
        }
    }

    pub fn poll(&self) -> Vec<GlobalAction> {
        let mut out = Vec::new();
        let Some(rx) = &self.event_rx else {
            return out;
        };
        while let Ok(ev) = rx.try_recv() {
            if ev.state != HotKeyState::Pressed {
                continue;
            }
            if let Some(t) = self.toggle_hotkey {
                if ev.id == t.id() {
                    out.push(GlobalAction::TogglePopup);
                }
            }
            if let Some(p) = self.play_hotkey {
                if ev.id == p.id() {
                    out.push(GlobalAction::TogglePlay);
                }
            }
        }
        out
    }
}

pub fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE")
        .map(|v| v.eq_ignore_ascii_case("wayland"))
        .unwrap_or(false)
}

pub fn is_gnome() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP")
        .map(|v| v.to_uppercase().contains("GNOME"))
        .unwrap_or(false)
}

fn current_exe_cmd() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "zenhush".to_string())
}

#[cfg(target_os = "linux")]
pub fn autostart_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("autostart").join("zenhush.desktop"))
}

#[cfg(not(target_os = "linux"))]
pub fn autostart_path() -> Option<PathBuf> {
    None
}

pub fn is_autostart_enabled() -> bool {
    autostart_path().map(|p| p.exists()).unwrap_or(false)
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let Some(path) = autostart_path() else {
        return Err("Autostart toggle is configured via OS Login Items on this platform".into());
    };
    if !enabled {
        if path.exists() {
            fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let exe = current_exe_cmd();
    let desktop = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Zenhush\n\
         Comment=Background focus soundscapes and local audio looper\n\
         Exec=\"{}\" --background\n\
         Icon=zenhush\n\
         Terminal=false\n\
         Categories=Audio;Utility;\n\
         X-GNOME-Autostart-enabled=true\n",
        exe
    );
    fs::write(&path, desktop).map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
fn write_icons_and_refresh_cache(data_dir: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let hicolor = data_dir.join("icons").join("hicolor");
    let dir_256 = hicolor.join("256x256").join("apps");
    let dir_svg = hicolor.join("scalable").join("apps");
    let dir_legacy_128 = hicolor.join("128x128").join("apps").join("zenhush.png");
    if dir_legacy_128.exists() {
        let _ = fs::remove_file(&dir_legacy_128);
    }

    for dir in [&hicolor, &hicolor.join("256x256"), &dir_256, &hicolor.join("scalable"), &dir_svg] {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o755));
    }

    let index_theme = hicolor.join("index.theme");
    if !index_theme.exists() {
        let sys_index = std::path::Path::new("/usr/share/icons/hicolor/index.theme");
        if sys_index.exists() {
            let _ = fs::copy(sys_index, &index_theme);
        }
    }

    let png_path = dir_256.join("zenhush.png");
    fs::write(&png_path, include_bytes!("../assets/icon.png")).map_err(|e| e.to_string())?;
    let _ = fs::set_permissions(&png_path, fs::Permissions::from_mode(0o644));

    let svg_path = dir_svg.join("zenhush.svg");
    fs::write(&svg_path, include_bytes!("../assets/zenhush.svg")).map_err(|e| e.to_string())?;
    let _ = fs::set_permissions(&svg_path, fs::Permissions::from_mode(0o644));

    let _ = std::process::Command::new("gtk-update-icon-cache")
        .args(["-f", "-t"])
        .arg(&hicolor)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    Ok(())
}

#[cfg(target_os = "linux")]
fn write_desktop_file(desktop_path: &std::path::Path, exec_bin: &str) -> Result<(), String> {
    let contents = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Zenhush\n\
         GenericName=Focus Audio Machine\n\
         Comment=Top-bar focus soundscapes & local audio loop\n\
         Exec=\"{}\" --toggle\n\
         Icon=zenhush\n\
         Terminal=false\n\
         Categories=AudioVideo;Audio;\n\
         StartupNotify=true\n\
         StartupWMClass=zenhush\n",
        exec_bin
    );
    fs::write(desktop_path, contents).map_err(|e| e.to_string())?;
    if let Some(apps) = desktop_path.parent() {
        let _ = std::process::Command::new("update-desktop-database")
            .arg(apps)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    Ok(())
}

/// Ensure icons and icon cache are up-to-date if a desktop launcher exists.
#[cfg(target_os = "linux")]
pub fn ensure_icon_assets_fresh() {
    let Some(data_dir) = dirs::data_local_dir() else {
        return;
    };
    let hicolor = data_dir.join("icons").join("hicolor");
    let svg = hicolor.join("scalable").join("apps").join("zenhush.svg");
    let png = hicolor.join("256x256").join("apps").join("zenhush.png");
    let desktop = data_dir.join("applications").join("zenhush.desktop");
    if desktop.exists() && (!svg.exists() || !png.exists()) {
        let _ = write_icons_and_refresh_cache(&data_dir);
    }
}

#[cfg(not(target_os = "linux"))]
pub fn ensure_icon_assets_fresh() {}

/// Install `~/.local/share/applications/zenhush.desktop` + icons on Linux.
#[cfg(target_os = "linux")]
pub fn install_desktop_entry() -> Result<PathBuf, String> {
    let data_dir = dirs::data_local_dir().ok_or("No local data dir")?;
    let apps = data_dir.join("applications");
    fs::create_dir_all(&apps).map_err(|e| e.to_string())?;
    write_icons_and_refresh_cache(&data_dir)?;

    let exe = current_exe_cmd();
    let desktop_path = apps.join("zenhush.desktop");
    write_desktop_file(&desktop_path, &exe)?;
    Ok(desktop_path)
}

#[cfg(not(target_os = "linux"))]
pub fn install_desktop_entry() -> Result<PathBuf, String> {
    Err("Desktop entry is Linux-only".into())
}

#[cfg(target_os = "linux")]
pub fn install_to_user_local() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let home = dirs::home_dir().ok_or("No home directory")?;
    let bin_dir = home.join(".local").join("bin");
    fs::create_dir_all(&bin_dir).map_err(|e| e.to_string())?;
    let target_bin = bin_dir.join("zenhush");
    if exe != target_bin {
        fs::copy(&exe, &target_bin).map_err(|e| format!("Failed copying binary: {}", e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&target_bin).map_err(|e| e.to_string())?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&target_bin, perms).map_err(|e| e.to_string())?;
        }
    }

    let data_dir = dirs::data_local_dir().ok_or("No local data dir")?;
    let apps = data_dir.join("applications");
    fs::create_dir_all(&apps).map_err(|e| e.to_string())?;
    write_icons_and_refresh_cache(&data_dir)?;

    let desktop_path = apps.join("zenhush.desktop");
    write_desktop_file(&desktop_path, &target_bin.to_string_lossy())?;

    Ok(format!(
        "Installed binary to {} and launcher to {}",
        target_bin.display(),
        desktop_path.display()
    ))
}

#[cfg(not(target_os = "linux"))]
pub fn install_to_user_local() -> Result<String, String> {
    Err("--install is currently Linux-only".into())
}

#[cfg(target_os = "linux")]
pub fn uninstall_from_user_local() -> Result<String, String> {
    let home = dirs::home_dir().ok_or("No home directory")?;
    let bin = home.join(".local").join("bin").join("zenhush");
    if bin.exists() {
        let _ = fs::remove_file(&bin);
    }
    if let Some(auto) = autostart_path() {
        if auto.exists() {
            let _ = fs::remove_file(&auto);
        }
    }
    if let Some(data_dir) = dirs::data_local_dir() {
        let apps = data_dir.join("applications");
        let desktop = apps.join("zenhush.desktop");
        if desktop.exists() {
            let _ = fs::remove_file(&desktop);
        }
        let hicolor = data_dir.join("icons").join("hicolor");
        for rel in [
            "128x128/apps/zenhush.png",
            "256x256/apps/zenhush.png",
            "scalable/apps/zenhush.svg",
        ] {
            let p = hicolor.join(rel);
            if p.exists() {
                let _ = fs::remove_file(&p);
            }
        }
        let _ = std::process::Command::new("gtk-update-icon-cache")
            .args(["-f", "-t"])
            .arg(&hicolor)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        let _ = std::process::Command::new("update-desktop-database")
            .arg(&apps)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    Ok("Removed ~/.local/bin/zenhush, desktop launcher, icon, and autostart".to_string())
}

#[cfg(not(target_os = "linux"))]
pub fn uninstall_from_user_local() -> Result<String, String> {
    Err("--uninstall is currently Linux-only".into())
}

/// Convert `"Alt+Shift+Z"` -> `"<Alt><Shift>z"` for GNOME gsettings.
pub fn to_gnome_accel(shortcut: &str) -> String {
    let mut mods = String::new();
    let mut key = String::new();
    for part in shortcut.split('+').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        match part.to_uppercase().as_str() {
            "ALT" | "OPTION" => mods.push_str("<Alt>"),
            "CTRL" | "CONTROL" => mods.push_str("<Control>"),
            "SHIFT" => mods.push_str("<Shift>"),
            "SUPER" | "META" | "CMD" => mods.push_str("<Super>"),
            "SPACE" => key = "space".to_string(),
            other => key = other.to_lowercase(),
        }
    }
    format!("{}{}", mods, key)
}

#[cfg(target_os = "linux")]
pub fn register_gnome_shortcuts(toggle_shortcut: &str, play_shortcut: &str) -> Result<(), String> {
    use std::process::Command;
    let _ = install_desktop_entry();
    let exe = current_exe_cmd();

    let toggle_path = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/zenhush-toggle/";
    let play_path = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/zenhush-play/";

    // Read existing custom-keybindings array
    let existing_out = Command::new("gsettings")
        .args([
            "get",
            "org.gnome.settings-daemon.plugins.media-keys",
            "custom-keybindings",
        ])
        .output()
        .map_err(|e| format!("gsettings failed: {}", e))?;
    let existing = String::from_utf8_lossy(&existing_out.stdout);

    let mut paths: Vec<String> = Vec::new();
    for item in existing.split('\'') {
        if item.starts_with('/') && !paths.contains(&item.to_string()) {
            paths.push(item.to_string());
        }
    }
    for p in [toggle_path, play_path] {
        if !paths.iter().any(|x| x == p) {
            paths.push(p.to_string());
        }
    }

    let array_literal = format!(
        "[{}]",
        paths
            .iter()
            .map(|p| format!("'{}'", p))
            .collect::<Vec<_>>()
            .join(", ")
    );

    let set_kv = |schema_path: &str, name: &str, cmd: &str, binding: &str| -> Result<(), String> {
        let schema = format!(
            "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding:{}",
            schema_path
        );
        for (k, v) in [("name", name), ("command", cmd), ("binding", binding)] {
            let status = Command::new("gsettings")
                .args(["set", &schema, k, v])
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err(format!("gsettings set {} failed", k));
            }
        }
        Ok(())
    };

    set_kv(
        toggle_path,
        "Toggle Zenhush",
        &format!("\"{}\" --toggle", exe),
        &to_gnome_accel(toggle_shortcut),
    )?;
    set_kv(
        play_path,
        "Zenhush Play/Pause",
        &format!("\"{}\" --play-pause", exe),
        &to_gnome_accel(play_shortcut),
    )?;

    let status = Command::new("gsettings")
        .args([
            "set",
            "org.gnome.settings-daemon.plugins.media-keys",
            "custom-keybindings",
            &array_literal,
        ])
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("Failed to update GNOME custom-keybindings list".into());
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn register_gnome_shortcuts(_toggle_shortcut: &str, _play_shortcut: &str) -> Result<(), String> {
    Err("GNOME keybindings are Linux-only".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gnome_accel_formatting() {
        assert_eq!(to_gnome_accel("Alt+Shift+Z"), "<Alt><Shift>z");
        assert_eq!(to_gnome_accel("Ctrl+Shift+Space"), "<Control><Shift>space");
        assert_eq!(to_gnome_accel("Super+Z"), "<Super>z");
    }
}
