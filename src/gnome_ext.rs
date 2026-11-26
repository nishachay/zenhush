//! Optional native GNOME Shell Top-Bar Pill extension (`zenhush@zenhush.app`).
//!
//! On GNOME Wayland, regular client windows cannot embed directly inside the
//! top panel. This tiny zero-dependency ESM extension (GNOME Shell 45–50)
//! adds a live Zenhush pill directly to the GNOME top bar driven by Zenhush's
//! MPRIS D-Bus interface:
//! - Shows a warm amber dot when playing + current sound/track name
//! - Left-click: opens/toggles Zenhush (`Raise` / `zenhush --toggle`)
//! - Middle-click: instant Play / Pause
//! - Mouse wheel scroll: Volume up / down

#[cfg(target_os = "linux")]
mod linux {
    use std::fs;
    use std::path::PathBuf;
    use std::process::Command;

    const UUID: &str = "zenhush@zenhush.app";

    const METADATA_JSON: &str = r#"{
  "uuid": "zenhush@zenhush.app",
  "name": "Zenhush Top-Bar Pill",
  "description": "Live focus audio pill in the GNOME top bar (left-click toggle, middle-click play/pause, scroll volume).",
  "shell-version": ["45", "46", "47", "48", "49", "50"],
  "version": 1
}
"#;

    const STYLESHEET_CSS: &str = r#".zenhush-pill {
  padding: 2px 10px;
  border-radius: 99px;
  background-color: rgba(28, 24, 20, 0.55);
  border: 1px solid rgba(255, 255, 255, 0.12);
  margin: 2px 4px;
}
.zenhush-pill.playing {
  background-color: rgba(200, 130, 62, 0.22);
  border-color: rgba(200, 130, 62, 0.55);
}
.zenhush-dot {
  width: 7px;
  height: 7px;
  border-radius: 99px;
  background-color: rgba(168, 159, 146, 0.7);
  margin-right: 6px;
}
.zenhush-pill.playing .zenhush-dot {
  background-color: #E2A86C;
}
.zenhush-label {
  font-size: 12px;
  font-weight: 600;
  color: #F5F1EA;
}
"#;

    const EXTENSION_JS: &str = r#"import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const BUS_NAME = 'org.mpris.MediaPlayer2.zenhush';
const OBJ_PATH = '/org/mpris/MediaPlayer2';
const PLAYER_IFACE = 'org.mpris.MediaPlayer2.Player';
const ROOT_IFACE = 'org.mpris.MediaPlayer2';

export default class ZenhushPillExtension extends Extension {
    enable() {
        this._btn = new PanelMenu.Button(0.5, 'Zenhush Pill', false);
        this._box = new St.BoxLayout({style_class: 'zenhush-pill', y_align: Clutter.ActorAlign.CENTER});
        this._dot = new St.Widget({style_class: 'zenhush-dot', y_align: Clutter.ActorAlign.CENTER});
        this._label = new St.Label({text: 'Zenhush', style_class: 'zenhush-label', y_align: Clutter.ActorAlign.CENTER});

        this._box.add_child(this._dot);
        this._box.add_child(this._label);
        this._btn.add_child(this._box);

        this._btn.connect('button-press-event', (_actor, event) => {
            const btn = event.get_button();
            if (btn === 2) {
                this._callPlayer('PlayPause');
                return Clutter.EVENT_STOP;
            }
            if (btn === 1 || btn === 3) {
                this._callRoot('Raise');
                return Clutter.EVENT_STOP;
            }
            return Clutter.EVENT_PROPAGATE;
        });

        this._btn.connect('scroll-event', (_actor, event) => {
            const dir = event.get_scroll_direction();
            let delta = 0;
            if (dir === Clutter.ScrollDirection.UP) delta = 0.05;
            else if (dir === Clutter.ScrollDirection.DOWN) delta = -0.05;
            if (delta !== 0) {
                this._adjustVolume(delta);
                return Clutter.EVENT_STOP;
            }
            return Clutter.EVENT_PROPAGATE;
        });

        Main.panel.addToStatusArea(this.uuid, this._btn, 0, 'center');

        this._propSub = Gio.DBus.session.signal_subscribe(
            BUS_NAME,
            'org.freedesktop.DBus.Properties',
            'PropertiesChanged',
            OBJ_PATH,
            null,
            Gio.DBusSignalFlags.NONE,
            () => this._refresh()
        );
        this._ownerSub = Gio.DBus.session.signal_subscribe(
            'org.freedesktop.DBus',
            'org.freedesktop.DBus',
            'NameOwnerChanged',
            '/org/freedesktop/DBus',
            BUS_NAME,
            Gio.DBusSignalFlags.NONE,
            () => this._refresh()
        );
        this._volume = 0.7;
        this._refresh();
    }

    _callPlayer(method) {
        Gio.DBus.session.call(BUS_NAME, OBJ_PATH, PLAYER_IFACE, method, null, null, Gio.DBusCallFlags.NONE, -1, null, null);
    }

    _callRoot(method) {
        Gio.DBus.session.call(BUS_NAME, OBJ_PATH, ROOT_IFACE, method, null, null, Gio.DBusCallFlags.NONE, -1, null, () => {});
    }

    _adjustVolume(delta) {
        const next = Math.max(0.0, Math.min(1.0, (this._volume || 0.7) + delta));
        this._volume = next;
        const val = new GLib.Variant('d', next);
        Gio.DBus.session.call(
            BUS_NAME, OBJ_PATH, 'org.freedesktop.DBus.Properties', 'Set',
            new GLib.Variant('(ssv)', [PLAYER_IFACE, 'Volume', val]),
            null, Gio.DBusCallFlags.NONE, -1, null, null
        );
    }

    _refresh() {
        Gio.DBus.session.call(
            BUS_NAME, OBJ_PATH, 'org.freedesktop.DBus.Properties', 'GetAll',
            new GLib.Variant('(s)', [PLAYER_IFACE]),
            null, Gio.DBusCallFlags.NONE, -1, null,
            (conn, res) => {
                try {
                    const [props] = conn.call_finish(res).deep_unpack();
                    const status = props.PlaybackStatus ? props.PlaybackStatus.deep_unpack() : 'Stopped';
                    const meta = props.Metadata ? props.Metadata.deep_unpack() : {};
                    const title = meta['xesam:title'] ? meta['xesam:title'].deep_unpack() : 'Zenhush';
                    if (props.Volume) this._volume = props.Volume.deep_unpack();
                    const playing = status === 'Playing';
                    this._btn.visible = true;
                    this._box.style_class = playing ? 'zenhush-pill playing' : 'zenhush-pill';
                    const shortTitle = title.length > 18 ? title.slice(0, 17) + '…' : title;
                    this._label.text = playing ? shortTitle : 'Zenhush';
                } catch (_e) {
                    this._btn.visible = false;
                }
            }
        );
    }

    disable() {
        if (this._propSub) Gio.DBus.session.signal_unsubscribe(this._propSub);
        if (this._ownerSub) Gio.DBus.session.signal_unsubscribe(this._ownerSub);
        if (this._btn) {
            this._btn.destroy();
            this._btn = null;
        }
    }
}
"#;

    fn ext_dir() -> Option<PathBuf> {
        dirs::data_local_dir().map(|d| d.join("gnome-shell").join("extensions").join(UUID))
    }

    pub fn is_installed() -> bool {
        ext_dir().map(|d| d.join("extension.js").exists()).unwrap_or(false)
    }

    pub fn install_gnome_pill() -> Result<String, String> {
        let dir = ext_dir().ok_or("No local data dir")?;
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        fs::write(dir.join("metadata.json"), METADATA_JSON).map_err(|e| e.to_string())?;
        fs::write(dir.join("stylesheet.css"), STYLESHEET_CSS).map_err(|e| e.to_string())?;
        // Ensure GLib is imported cleanly in extension.js
        let js = format!("import GLib from 'gi://GLib';\n{}", EXTENSION_JS);
        fs::write(dir.join("extension.js"), js).map_err(|e| e.to_string())?;

        let _ = Command::new("gnome-extensions").args(["enable", UUID]).status();
        Ok("Installed GNOME Top-Bar Pill (re-login or toggle Extensions if needed)".to_string())
    }

    pub fn uninstall_gnome_pill() -> Result<String, String> {
        let _ = Command::new("gnome-extensions").args(["disable", UUID]).status();
        if let Some(dir) = ext_dir() {
            if dir.exists() {
                fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
            }
        }
        Ok("Removed GNOME Top-Bar Pill extension".to_string())
    }
}

#[cfg(not(target_os = "linux"))]
mod fallback {
    pub fn is_installed() -> bool {
        false
    }
    pub fn install_gnome_pill() -> Result<String, String> {
        Err("GNOME Top-Bar Pill is Linux GNOME-only".into())
    }
    pub fn uninstall_gnome_pill() -> Result<String, String> {
        Err("GNOME Top-Bar Pill is Linux GNOME-only".into())
    }
}

#[cfg(target_os = "linux")]
pub use linux::{install_gnome_pill, is_installed, uninstall_gnome_pill};
#[cfg(not(target_os = "linux"))]
pub use fallback::{install_gnome_pill, is_installed, uninstall_gnome_pill};
