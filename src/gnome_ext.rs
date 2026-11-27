//! Native GNOME Shell Top-Bar Popover extension (`zenhush@zenhush.app`, GNOME 45–51).
//!
//! Renders the entire Zenhush experience directly inside the GNOME top bar
//! (native PanelMenu.Button + anchored PopupMenu), driven over D-Bus by the
//! lightweight ~15 MB Zenhush Core daemon. Zero floating OpenGL windows, zero
//! black-corner artifacts, zero laptop heating.

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    use zbus::zvariant::OwnedValue;

    const UUID: &str = "zenhush@zenhush.app";
    const EXT_VERSION: u32 = 6;

    const METADATA_JSON: &str = r#"{
  "uuid": "zenhush@zenhush.app",
  "name": "Zenhush",
  "description": "Minimal focus audio player directly in your GNOME top bar.",
  "shell-version": ["45", "46", "47", "48", "49", "50", "51"],
  "version": 6
}
"#;

    /// Crisp 3-bar harmonic waveform symbolic SVG icon for the GNOME top bar.
    const SYMBOLIC_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16">
  <rect x="2" y="5" width="2.6" height="6" rx="1.3" fill="#bebebe"/>
  <rect x="6.7" y="2" width="2.6" height="12" rx="1.3" fill="#bebebe"/>
  <rect x="11.4" y="4" width="2.6" height="8" rx="1.3" fill="#bebebe"/>
</svg>
"##;

    const STYLESHEET_CSS: &str = r#".zenhush-panel-pill {
  spacing: 6px;
  padding: 2px 9px;
  border-radius: 99px;
  background-color: rgba(255, 255, 255, 0.06);
  border: 1px solid rgba(255, 255, 255, 0.10);
  transition-duration: 120ms;
}
.zenhush-panel-pill.playing {
  background-color: rgba(200, 130, 62, 0.24);
  border-color: rgba(226, 168, 108, 0.55);
}
.zenhush-panel-icon {
  color: rgba(235, 229, 220, 0.88);
}
.zenhush-panel-pill.playing .zenhush-panel-icon {
  color: #E2A86C;
}
.zenhush-panel-dot {
  width: 6px;
  height: 6px;
  border-radius: 99px;
  background-color: #E2A86C;
}
.zenhush-panel-label {
  font-size: 12px;
  font-weight: 600;
}

.zenhush-menu-item {
  padding: 0;
  margin: 0;
}

.zenhush-menu-root {
  width: 308px;
  padding: 14px;
  spacing: 14px;
}

.zenhush-hero {
  padding: 12px;
  border-radius: 12px;
  background-color: rgba(255, 255, 255, 0.06);
  spacing: 12px;
}
.zenhush-title {
  font-size: 16px;
  font-weight: 700;
  color: #F5F1EA;
}
.zenhush-subtitle {
  font-size: 12px;
  color: #B4ABA0;
}
.zenhush-subtitle.error {
  color: #F2B880;
}

.zenhush-play-btn {
  width: 44px;
  height: 44px;
  border-radius: 99px;
  background-color: #C8823E;
  color: #14100C;
  transition-duration: 120ms;
}
.zenhush-play-btn:hover {
  background-color: #D7914D;
}
.zenhush-play-btn:active {
  background-color: #B47232;
}
.zenhush-play-btn:focus {
  box-shadow: 0 0 0 2px #F5F1EA;
}

.zenhush-grid-wrap {
  spacing: 6px;
}

.zenhush-chip {
  min-height: 32px;
  padding: 7px 6px;
  border-radius: 9px;
  font-size: 12px;
  font-weight: 600;
  color: #E2DBD0;
  background-color: rgba(255, 255, 255, 0.08);
  border: 1px solid rgba(255, 255, 255, 0.06);
  transition-duration: 120ms;
}
.zenhush-chip:hover {
  background-color: rgba(255, 255, 255, 0.14);
  color: #FFFFFF;
}
.zenhush-chip:focus {
  box-shadow: 0 0 0 2px #F5F1EA;
}
.zenhush-chip.active {
  background-color: #C8823E;
  border-color: #E6AE75;
  color: #14100C;
}
.zenhush-chip.active:hover {
  background-color: #D7914D;
  color: #14100C;
}

.zenhush-vol-row {
  spacing: 8px;
}
.zenhush-icon-btn {
  min-width: 28px;
  min-height: 28px;
  padding: 6px;
  border-radius: 8px;
  color: #D4CBC0;
  background-color: transparent;
  transition-duration: 120ms;
}
.zenhush-icon-btn:hover {
  background-color: rgba(255, 255, 255, 0.11);
  color: #FFFFFF;
}
.zenhush-icon-btn:focus {
  box-shadow: 0 0 0 2px #F5F1EA;
}
.zenhush-icon-btn.active {
  color: #E2A86C;
}
.zenhush-vol-hit {
  height: 28px;
}
.zenhush-vol-track {
  height: 5px;
  border-radius: 99px;
  background-color: rgba(255, 255, 255, 0.16);
}
.zenhush-vol-fill {
  height: 5px;
  border-radius: 99px;
  background-color: #C8823E;
}
.zenhush-vol-fill.muted {
  background-color: rgba(176, 167, 154, 0.45);
}
.zenhush-vol-knob {
  width: 13px;
  height: 13px;
  border-radius: 99px;
  background-color: #F5F1EA;
  border: 2px solid #C8823E;
}
.zenhush-vol-knob.muted {
  border-color: #7D7468;
}
.zenhush-vol-pct {
  font-size: 12px;
  font-weight: 600;
  font-feature-settings: "tnum";
  color: #B4ABA0;
  min-width: 38px;
  text-align: right;
}

.zenhush-action-pill {
  min-height: 32px;
  padding: 7px 10px;
  border-radius: 9px;
  font-size: 12px;
  font-weight: 600;
  font-feature-settings: "tnum";
  color: #EAE3D8;
  background-color: rgba(255, 255, 255, 0.08);
  border: 1px solid rgba(255, 255, 255, 0.06);
  transition-duration: 120ms;
}
.zenhush-action-pill:hover {
  background-color: rgba(255, 255, 255, 0.14);
}
.zenhush-action-pill:focus {
  box-shadow: 0 0 0 2px #F5F1EA;
}
.zenhush-action-pill.active {
  background-color: #C8823E;
  border-color: #E6AE75;
  color: #14100C;
}

/* Concentric radii: outer 10px = inner 5px + padding 5px */
.zenhush-file-row {
  spacing: 6px;
  padding: 5px 8px;
  border-radius: 10px;
  background-color: rgba(255, 255, 255, 0.06);
  border: 1px solid rgba(255, 255, 255, 0.08);
  transition-duration: 120ms;
}
.zenhush-file-row.active {
  border-color: #E2A86C;
  background-color: rgba(200, 130, 62, 0.16);
}
.zenhush-file-name {
  min-height: 26px;
  padding: 3px 4px;
  border-radius: 5px;
  font-size: 12px;
  font-weight: 600;
  color: #F5F1EA;
}
.zenhush-file-name:focus {
  box-shadow: 0 0 0 2px #F5F1EA;
}
.zenhush-lock-pill {
  min-height: 26px;
  padding: 4px 9px;
  border-radius: 5px;
  font-size: 11px;
  font-weight: 600;
  color: #E2DBD0;
  background-color: rgba(255, 255, 255, 0.10);
  transition-duration: 120ms;
}
.zenhush-lock-pill:hover {
  background-color: rgba(255, 255, 255, 0.18);
}
.zenhush-lock-pill:focus {
  box-shadow: 0 0 0 2px #F5F1EA;
}
.zenhush-lock-pill.locked {
  background-color: #C8823E;
  color: #14100C;
}

.zenhush-footer {
  spacing: 8px;
}
.zenhush-footer-link {
  min-height: 28px;
  padding: 5px 8px;
  border-radius: 7px;
  font-size: 12px;
  font-weight: 500;
  color: #B4ABA0;
  transition-duration: 120ms;
}
.zenhush-footer-link:hover {
  color: #F5F1EA;
  background-color: rgba(255, 255, 255, 0.09);
}
.zenhush-footer-link:focus {
  box-shadow: 0 0 0 2px #F5F1EA;
}
"#;

    const EXTENSION_JS: &str = r#"import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const BUS_NAME = 'org.mpris.MediaPlayer2.zenhush';
const OBJ_PATH = '/org/mpris/MediaPlayer2';
const PLAYER_IFACE = 'org.mpris.MediaPlayer2.Player';
const CTRL_IFACE = 'org.zenhush.Control';
const PROPS_IFACE = 'org.freedesktop.DBus.Properties';

const SOUNDS = ['Brown', 'Pink', 'White', 'Rain', 'Ocean', 'Fire', 'Forest', 'Focus'];

function fmtTime(secs) {
    const s = Math.max(0, Math.floor(secs));
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const r = s % 60;
    if (h > 0) return `${h}:${String(m).padStart(2, '0')}:${String(r).padStart(2, '0')}`;
    return `${m}:${String(r).padStart(2, '0')}`;
}

function trunc(str, max) {
    if (!str) return '';
    return str.length > max ? str.slice(0, max - 1) + '…' : str;
}

const ZenhushIndicator = GObject.registerClass(
class ZenhushIndicator extends PanelMenu.Button {
    _init(extPath) {
        super._init(0.5, 'Zenhush', false);

        this._state = {
            playing: false,
            has_played: false,
            is_file: false,
            sound_idx: 0,
            title: 'Brown Noise',
            subtitle: 'deep and warm',
            volume: 70,
            muted: false,
            timer_minutes: 0,
            timer_remaining_secs: null,
            has_file: false,
            file_title: '',
            locked_default: false,
            autostart: false,
            audio_error: null,
        };
        this._draggingVol = false;

        this._pill = new St.BoxLayout({
            style_class: 'zenhush-panel-pill',
            y_align: Clutter.ActorAlign.CENTER,
        });
        const iconFile = Gio.File.new_for_path(`${extPath}/zenhush-symbolic.svg`);
        this._panelIcon = new St.Icon({
            gicon: Gio.FileIcon.new(iconFile),
            icon_size: 14,
            style_class: 'zenhush-panel-icon',
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._dot = new St.Widget({
            style_class: 'zenhush-panel-dot',
            y_align: Clutter.ActorAlign.CENTER,
            visible: false,
        });
        this._panelLabel = new St.Label({
            text: 'Zenhush',
            style_class: 'zenhush-panel-label',
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._pill.add_child(this._panelIcon);
        this._pill.add_child(this._dot);
        this._pill.add_child(this._panelLabel);
        this.add_child(this._pill);

        // Middle-click = instant Play/Pause (no scroll-wheel volume jumps)
        this.connect('button-press-event', (_actor, event) => {
            if (event.get_button() === 2) {
                this._callPlayer('PlayPause');
                return Clutter.EVENT_STOP;
            }
            return Clutter.EVENT_PROPAGATE;
        });

        this._buildMenu();
        this.visible = false;
    }

    _buildMenu() {
        const containerItem = new PopupMenu.PopupBaseMenuItem({
            reactive: false,
            can_focus: false,
            style_class: 'zenhush-menu-item',
        });
        containerItem.track_hover = false;

        const root = new St.BoxLayout({
            vertical: true,
            style_class: 'zenhush-menu-root',
        });

        // 1. Hero Card: Title/Subtitle + Big Play Button
        const hero = new St.BoxLayout({
            style_class: 'zenhush-hero',
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
        });
        const heroText = new St.BoxLayout({
            vertical: true,
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._titleLabel = new St.Label({text: 'Brown Noise', style_class: 'zenhush-title'});
        this._subtitleLabel = new St.Label({text: 'deep and warm · Ready', style_class: 'zenhush-subtitle'});
        heroText.add_child(this._titleLabel);
        heroText.add_child(this._subtitleLabel);

        this._playIcon = new St.Icon({
            icon_name: 'media-playback-start-symbolic',
            icon_size: 18,
            translation_x: 1.5,
        });
        this._playBtn = new St.Button({
            style_class: 'zenhush-play-btn',
            child: this._playIcon,
            can_focus: true,
            accessible_name: 'Play focus audio',
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._playBtn.connect('clicked', () => this._callPlayer('PlayPause'));

        hero.add_child(heroText);
        hero.add_child(this._playBtn);
        root.add_child(hero);

        // 2. Homogeneous 4x2 Sound Grid (tight 6px internal gap vs 14px external group gap)
        const gridWrap = new St.BoxLayout({
            vertical: true,
            style_class: 'zenhush-grid-wrap',
            x_expand: true,
        });
        this._soundBtns = [];
        for (let r = 0; r < 2; r++) {
            const row = new St.Widget({
                x_expand: true,
                layout_manager: new Clutter.BoxLayout({
                    orientation: Clutter.Orientation.HORIZONTAL,
                    homogeneous: true,
                    spacing: 6,
                }),
            });
            for (let c = 0; c < 4; c++) {
                const idx = r * 4 + c;
                const btn = new St.Button({
                    label: SOUNDS[idx],
                    style_class: 'zenhush-chip',
                    can_focus: true,
                    x_expand: true,
                });
                btn.connect('clicked', () => {
                    this._callCtrl('SelectSound', new GLib.Variant('(u)', [idx]));
                });
                this._soundBtns.push(btn);
                row.add_child(btn);
            }
            gridWrap.add_child(row);
        }
        root.add_child(gridWrap);

        // 3. Volume Row (28px hit target + unclipped thumb knob + tnum %)
        const volRow = new St.BoxLayout({
            style_class: 'zenhush-vol-row',
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._muteIcon = new St.Icon({
            icon_name: 'audio-volume-high-symbolic',
            icon_size: 15,
        });
        this._muteBtn = new St.Button({
            style_class: 'zenhush-icon-btn',
            child: this._muteIcon,
            can_focus: true,
            accessible_name: 'Mute audio',
        });
        this._muteBtn.connect('clicked', () => this._callCtrl('ToggleMute', null));

        this._volHit = new St.Widget({
            style_class: 'zenhush-vol-hit',
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
            reactive: true,
            layout_manager: new Clutter.BinLayout(),
        });
        this._volTrack = new St.Widget({
            style_class: 'zenhush-vol-track',
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._volFill = new St.Widget({
            style_class: 'zenhush-vol-fill',
            x_align: Clutter.ActorAlign.START,
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._volKnob = new St.Widget({
            style_class: 'zenhush-vol-knob',
            x_align: Clutter.ActorAlign.START,
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._volHit.add_child(this._volTrack);
        this._volHit.add_child(this._volFill);
        this._volHit.add_child(this._volKnob);

        this._volHit.connect('notify::allocation', () => {
            this._updateVolumeBar(this._state.volume ?? 70, Boolean(this._state.muted));
        });
        this._volHit.connect('button-press-event', (_a, ev) => {
            this._draggingVol = true;
            this._applyVolPointer(ev);
            return Clutter.EVENT_STOP;
        });
        this._volHit.connect('motion-event', (_a, ev) => {
            if (this._draggingVol) {
                this._applyVolPointer(ev);
                return Clutter.EVENT_STOP;
            }
            return Clutter.EVENT_PROPAGATE;
        });
        this._volHit.connect('button-release-event', () => {
            this._draggingVol = false;
            return Clutter.EVENT_STOP;
        });
        this._volHit.connect('leave-event', () => {
            this._draggingVol = false;
            return Clutter.EVENT_PROPAGATE;
        });

        this._volPctLabel = new St.Label({
            text: '70%',
            style_class: 'zenhush-vol-pct',
            y_align: Clutter.ActorAlign.CENTER,
        });

        volRow.add_child(this._muteBtn);
        volRow.add_child(this._volHit);
        volRow.add_child(this._volPctLabel);
        root.add_child(volRow);

        // 4. Homogeneous 2-Column Actions Row: Timer + Choose File…
        const actRow = new St.Widget({
            x_expand: true,
            layout_manager: new Clutter.BoxLayout({
                orientation: Clutter.Orientation.HORIZONTAL,
                homogeneous: true,
                spacing: 8,
            }),
        });
        this._timerBtn = new St.Button({
            label: 'Timer: Off',
            style_class: 'zenhush-action-pill',
            can_focus: true,
            x_expand: true,
        });
        this._timerBtn.connect('clicked', () => this._callCtrl('CycleTimer', null));

        this._pickFileBtn = new St.Button({
            label: '+ My Audio…',
            style_class: 'zenhush-action-pill',
            can_focus: true,
            x_expand: true,
        });
        this._pickFileBtn.connect('clicked', () => {
            this.menu.close();
            this._callCtrl('PickFile', null);
        });

        actRow.add_child(this._timerBtn);
        actRow.add_child(this._pickFileBtn);
        root.add_child(actRow);

        // 5. Sleek Local Track Pill (only visible when user has chosen a local file)
        this._fileRow = new St.BoxLayout({
            style_class: 'zenhush-file-row',
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
            visible: false,
        });
        this._fileSelectBtn = new St.Button({
            label: 'Local track',
            style_class: 'zenhush-file-name',
            can_focus: true,
            x_expand: true,
            x_align: Clutter.ActorAlign.START,
        });
        this._fileSelectBtn.connect('clicked', () => {
            if (this._state.has_file) {
                this._callCtrl('SelectFile', null);
            } else {
                this.menu.close();
                this._callCtrl('PickFile', null);
            }
        });

        this._lockIcon = new St.Icon({
            icon_name: 'changes-allow-symbolic',
            icon_size: 12,
        });
        const lockContent = new St.BoxLayout({
            style: 'spacing: 5px;',
            y_align: Clutter.ActorAlign.CENTER,
        });
        lockContent.add_child(this._lockIcon);
        this._lockLabel = new St.Label({
            text: 'Default',
            y_align: Clutter.ActorAlign.CENTER,
        });
        lockContent.add_child(this._lockLabel);

        this._lockBtn = new St.Button({
            child: lockContent,
            style_class: 'zenhush-lock-pill',
            can_focus: true,
        });
        this._lockBtn.connect('clicked', () => {
            if (this._state.has_file) {
                this._callCtrl('ToggleLockDefault', null);
            } else {
                this.menu.close();
                this._callCtrl('PickAndLockFile', null);
            }
        });

        const clearIcon = new St.Icon({icon_name: 'window-close-symbolic', icon_size: 13});
        this._clearFileBtn = new St.Button({
            style_class: 'zenhush-icon-btn',
            child: clearIcon,
            can_focus: true,
            accessible_name: 'Remove local file',
        });
        this._clearFileBtn.connect('clicked', () => this._callCtrl('RemoveFile', null));

        this._fileRow.add_child(this._fileSelectBtn);
        this._fileRow.add_child(this._lockBtn);
        this._fileRow.add_child(this._clearFileBtn);
        root.add_child(this._fileRow);

        // 6. Quiet Footer (separated by 14px space, zero extra divider lines)
        const footer = new St.BoxLayout({style_class: 'zenhush-footer', x_expand: true});
        this._autostartBtn = new St.Button({
            label: 'Start at login: Off',
            style_class: 'zenhush-footer-link',
            can_focus: true,
            x_expand: true,
            x_align: Clutter.ActorAlign.START,
        });
        this._autostartBtn.connect('clicked', () => this._callCtrl('ToggleAutostart', null));

        const quitBtn = new St.Button({
            label: 'Quit Zenhush',
            style_class: 'zenhush-footer-link',
            can_focus: true,
            x_align: Clutter.ActorAlign.END,
        });
        quitBtn.connect('clicked', () => {
            this.menu.close();
            this._callCtrl('Quit', null);
        });

        footer.add_child(this._autostartBtn);
        footer.add_child(quitBtn);
        root.add_child(footer);

        containerItem.add_child(root);
        this.menu.addMenuItem(containerItem);
    }

    _applyVolPointer(event) {
        const [ex] = event.get_coords();
        const [tx] = this._volHit.get_transformed_position();
        const knobW = 13;
        const usable = Math.max(1, this._volHit.width - knobW);
        const pct = Math.round(Math.max(0, Math.min(1, (ex - tx - knobW * 0.5) / usable)) * 100);
        this._updateVolumeBar(pct, false);
        this._callCtrl('SetVolumePct', new GLib.Variant('(y)', [pct]));
    }

    _updateVolumeBar(pct, muted) {
        const w = this._volHit.width;
        if (w <= 1) return;
        const knobW = 13;
        const clamped = muted ? 0 : Math.max(0, Math.min(100, pct));
        const knobX = Math.round(((w - knobW) * clamped) / 100);
        this._volFill.width = Math.max(knobW * 0.5, knobX + knobW * 0.5);
        this._volFill.style_class = muted ? 'zenhush-vol-fill muted' : 'zenhush-vol-fill';
        this._volKnob.translation_x = knobX;
        this._volKnob.style_class = muted ? 'zenhush-vol-knob muted' : 'zenhush-vol-knob';
        this._volPctLabel.text = muted ? 'Mute' : `${pct}%`;
        this._muteIcon.icon_name = muted ? 'audio-volume-muted-symbolic' : 'audio-volume-high-symbolic';
        this._muteBtn.accessible_name = muted ? 'Unmute audio' : 'Mute audio';
        this._muteBtn.style_class = muted ? 'zenhush-icon-btn active' : 'zenhush-icon-btn';
    }

    applyState(st) {
        this._state = st;
        this.visible = true;

        // Top bar pill
        this._pill.style_class = st.playing ? 'zenhush-panel-pill playing' : 'zenhush-panel-pill';
        this._dot.visible = Boolean(st.playing);
        if (st.playing) {
            const rem = st.timer_remaining_secs;
            const titleShort = trunc(st.title, 15);
            this._panelLabel.text = rem && rem > 0 ? `${titleShort} · ${fmtTime(rem)}` : titleShort;
        } else {
            this._panelLabel.text = 'Zenhush';
        }

        // Hero (show full title on hover if truncated)
        this._titleLabel.text = trunc(st.title, 19);
        this._titleLabel.has_tooltip = st.title && st.title.length > 19;
        this._titleLabel.tooltip_text = st.title || '';
        if (st.audio_error) {
            this._subtitleLabel.text = trunc(st.audio_error, 30);
            this._subtitleLabel.style_class = 'zenhush-subtitle error';
        } else {
            const stateStr = st.playing ? 'Playing' : (st.has_played ? 'Paused' : 'Ready');
            this._subtitleLabel.text = `${st.subtitle} · ${stateStr}`;
            this._subtitleLabel.style_class = 'zenhush-subtitle';
        }
        this._playIcon.icon_name = st.playing
            ? 'media-playback-pause-symbolic'
            : 'media-playback-start-symbolic';
        // Optical nudge toward triangle tip when showing Play icon
        this._playIcon.translation_x = st.playing ? 0 : 1.5;
        this._playBtn.accessible_name = st.playing ? 'Pause focus audio' : 'Play focus audio';

        // Sound Grid
        for (let i = 0; i < this._soundBtns.length; i++) {
            const active = !st.is_file && st.sound_idx === i;
            this._soundBtns[i].style_class = active ? 'zenhush-chip active' : 'zenhush-chip';
        }

        // Volume
        this._updateVolumeBar(st.volume, st.muted);

        // Timer button
        if (st.timer_remaining_secs && st.timer_remaining_secs > 0) {
            this._timerBtn.label = `Timer: ${fmtTime(st.timer_remaining_secs)}`;
            this._timerBtn.style_class = 'zenhush-action-pill active';
        } else if (st.timer_minutes > 0) {
            this._timerBtn.label = `Timer: ${st.timer_minutes}m`;
            this._timerBtn.style_class = 'zenhush-action-pill active';
        } else {
            this._timerBtn.label = 'Timer: Off';
            this._timerBtn.style_class = 'zenhush-action-pill';
        }

        // File row (shown only when a local track is chosen — zero empty box clutter)
        this._pickFileBtn.label = st.has_file ? 'Change File…' : '+ My Audio…';
        this._fileRow.visible = Boolean(st.has_file);
        this._clearFileBtn.visible = Boolean(st.has_file);
        if (st.has_file) {
            this._fileRow.style_class = st.is_file ? 'zenhush-file-row active' : 'zenhush-file-row';
            const fullName = st.file_title || 'Local track';
            this._fileSelectBtn.label = trunc(fullName, 17);
            this._fileSelectBtn.has_tooltip = true;
            this._fileSelectBtn.tooltip_text = `Play ${fullName}`;
            this._lockIcon.icon_name = st.locked_default
                ? 'changes-prevent-symbolic'
                : 'changes-allow-symbolic';
            this._lockBtn.style_class = st.locked_default ? 'zenhush-lock-pill locked' : 'zenhush-lock-pill';
            this._lockBtn.has_tooltip = true;
            this._lockBtn.tooltip_text = st.locked_default
                ? 'Locked: always opens Zenhush with this track on launch'
                : 'Lock as startup default even after playing other sounds';
        }

        // Footer
        this._autostartBtn.label = st.autostart ? 'Start at login: On' : 'Start at login: Off';
    }

    _callPlayer(method) {
        Gio.DBus.session.call(
            BUS_NAME, OBJ_PATH, PLAYER_IFACE, method,
            null, null, Gio.DBusCallFlags.NONE, -1, null, null
        );
    }

    _callCtrl(method, params) {
        Gio.DBus.session.call(
            BUS_NAME, OBJ_PATH, CTRL_IFACE, method,
            params, null, Gio.DBusCallFlags.NONE, -1, null, null
        );
    }
});

export default class ZenhushTopBarExtension extends Extension {
    enable() {
        this._indicator = new ZenhushIndicator(this.path);
        Main.panel.addToStatusArea(this.uuid, this._indicator, 0, 'right');

        this._propSub = Gio.DBus.session.signal_subscribe(
            BUS_NAME,
            PROPS_IFACE,
            'PropertiesChanged',
            OBJ_PATH,
            null,
            Gio.DBusSignalFlags.NONE,
            () => this._refresh()
        );
        this._toggleSub = Gio.DBus.session.signal_subscribe(
            BUS_NAME,
            CTRL_IFACE,
            'TogglePopover',
            OBJ_PATH,
            null,
            Gio.DBusSignalFlags.NONE,
            () => {
                if (this._indicator) {
                    this._indicator.visible = true;
                    this._indicator.menu.toggle();
                }
            }
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

        this._menuOpenSub = this._indicator.menu.connect('open-state-changed', (_m, open) => {
            if (open) this._refresh();
        });

        this._refresh();
    }

    _refresh() {
        Gio.DBus.session.call(
            BUS_NAME,
            OBJ_PATH,
            PROPS_IFACE,
            'Get',
            new GLib.Variant('(ss)', [CTRL_IFACE, 'StateJson']),
            null,
            Gio.DBusCallFlags.NONE,
            -1,
            null,
            (conn, res) => {
                if (!this._indicator) return;
                try {
                    const [variant] = conn.call_finish(res).deep_unpack();
                    const jsonStr = variant.deep_unpack();
                    const st = JSON.parse(jsonStr);
                    this._indicator.applyState(st);
                } catch (_e) {
                    this._indicator.visible = false;
                }
            }
        );
    }

    disable() {
        if (this._propSub) Gio.DBus.session.signal_unsubscribe(this._propSub);
        if (this._toggleSub) Gio.DBus.session.signal_unsubscribe(this._toggleSub);
        if (this._ownerSub) Gio.DBus.session.signal_unsubscribe(this._ownerSub);
        if (this._indicator) {
            if (this._menuOpenSub) this._indicator.menu.disconnect(this._menuOpenSub);
            this._indicator.destroy();
            this._indicator = null;
        }
    }
}
"#;

    fn ext_dir() -> Option<PathBuf> {
        dirs::data_local_dir().map(|d| d.join("gnome-shell").join("extensions").join(UUID))
    }

    pub fn is_installed() -> bool {
        ext_dir()
            .map(|d| d.join("extension.js").exists())
            .unwrap_or(false)
    }

    fn installed_version() -> u32 {
        let Some(dir) = ext_dir() else {
            return 0;
        };
        let Ok(text) = fs::read_to_string(dir.join("metadata.json")) else {
            return 0;
        };
        serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.get("version")?.as_u64())
            .map(|n| n as u32)
            .unwrap_or(0)
    }

    pub fn is_active() -> bool {
        let Ok(conn) = zbus::blocking::Connection::session() else {
            return false;
        };
        let Ok(msg) = conn.call_method(
            Some("org.gnome.Shell.Extensions"),
            "/org/gnome/Shell/Extensions",
            Some("org.gnome.Shell.Extensions"),
            "GetExtensionInfo",
            &(UUID,),
        ) else {
            return false;
        };
        let Ok(info) = msg.body().deserialize::<HashMap<String, OwnedValue>>() else {
            return false;
        };
        if let Some(state_val) = info.get("state") {
            if let Ok(state) = f64::try_from(state_val.clone()) {
                return (state - 1.0).abs() < 0.01;
            }
            if let Ok(state) = i32::try_from(state_val.clone()) {
                return state == 1;
            }
        }
        false
    }

    pub fn ensure_installed_and_enabled() {
        let needs_write = !is_installed() || installed_version() < EXT_VERSION;
        if needs_write {
            let _ = install_gnome_pill();
        } else if !is_active() {
            let _ = enable_extension_dbus();
        }
    }

    fn ensure_gsettings_enabled() {
        let Ok(out) = Command::new("gsettings")
            .args(["get", "org.gnome.shell", "enabled-extensions"])
            .output()
        else {
            return;
        };
        let raw = String::from_utf8_lossy(&out.stdout);
        let mut items: Vec<String> = raw
            .split('\'')
            .enumerate()
            .filter_map(|(i, s)| {
                if i % 2 == 1 {
                    Some(s.to_string())
                } else {
                    None
                }
            })
            .collect();
        if !items.iter().any(|x| x == UUID) {
            items.push(UUID.to_string());
            let literal = format!(
                "[{}]",
                items
                    .iter()
                    .map(|x| format!("'{}'", x))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let _ = Command::new("gsettings")
                .args(["set", "org.gnome.shell", "enabled-extensions", &literal])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }

    fn enable_extension_dbus() -> bool {
        ensure_gsettings_enabled();
        if let Ok(conn) = zbus::blocking::Connection::session() {
            let _ = conn.call_method(
                Some("org.gnome.Shell.Extensions"),
                "/org/gnome/Shell/Extensions",
                Some("org.gnome.Shell.Extensions"),
                "ReloadExtension",
                &(UUID,),
            );
            if let Ok(reply) = conn.call_method(
                Some("org.gnome.Shell.Extensions"),
                "/org/gnome/Shell/Extensions",
                Some("org.gnome.Shell.Extensions"),
                "EnableExtension",
                &(UUID,),
            ) {
                if let Ok(ok) = reply.body().deserialize::<bool>() {
                    if ok {
                        return true;
                    }
                }
            }
        }
        Command::new("gnome-extensions")
            .args(["enable", UUID])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    pub fn install_gnome_pill() -> Result<String, String> {
        let dir = ext_dir().ok_or("No local data dir")?;
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        fs::write(dir.join("metadata.json"), METADATA_JSON).map_err(|e| e.to_string())?;
        fs::write(dir.join("stylesheet.css"), STYLESHEET_CSS).map_err(|e| e.to_string())?;
        fs::write(dir.join("extension.js"), EXTENSION_JS).map_err(|e| e.to_string())?;
        fs::write(dir.join("zenhush-symbolic.svg"), SYMBOLIC_SVG).map_err(|e| e.to_string())?;

        let enabled = enable_extension_dbus();
        if enabled && is_active() {
            Ok("GNOME Top-Bar Popover active right in your top bar!".to_string())
        } else {
            Ok("Installed GNOME Top-Bar Popover (log out & back in once to activate)".to_string())
        }
    }

    pub fn uninstall_gnome_pill() -> Result<String, String> {
        let _ = Command::new("gnome-extensions")
            .args(["disable", UUID])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if let Ok(out) = Command::new("gsettings")
            .args(["get", "org.gnome.shell", "enabled-extensions"])
            .output()
        {
            let raw = String::from_utf8_lossy(&out.stdout);
            let filtered: Vec<String> = raw
                .split('\'')
                .enumerate()
                .filter_map(|(i, s)| {
                    if i % 2 == 1 {
                        Some(s.to_string())
                    } else {
                        None
                    }
                })
                .filter(|x| x != UUID)
                .collect();
            let literal = format!(
                "[{}]",
                filtered
                    .iter()
                    .map(|x| format!("'{}'", x))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let _ = Command::new("gsettings")
                .args(["set", "org.gnome.shell", "enabled-extensions", &literal])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        if let Some(dir) = ext_dir() {
            if dir.exists() {
                fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
            }
        }
        Ok("Removed GNOME Top-Bar Popover extension".to_string())
    }
}

#[cfg(not(target_os = "linux"))]
mod fallback {
    pub fn is_installed() -> bool {
        false
    }
    pub fn is_active() -> bool {
        false
    }
    pub fn ensure_installed_and_enabled() {}
    pub fn install_gnome_pill() -> Result<String, String> {
        Err("GNOME Top-Bar Popover is Linux GNOME-only".into())
    }
    pub fn uninstall_gnome_pill() -> Result<String, String> {
        Err("GNOME Top-Bar Popover is Linux GNOME-only".into())
    }
}

#[cfg(not(target_os = "linux"))]
pub use fallback::{
    ensure_installed_and_enabled, install_gnome_pill, is_active, is_installed, uninstall_gnome_pill,
};
#[cfg(target_os = "linux")]
pub use linux::{
    ensure_installed_and_enabled, install_gnome_pill, is_active, is_installed, uninstall_gnome_pill,
};
