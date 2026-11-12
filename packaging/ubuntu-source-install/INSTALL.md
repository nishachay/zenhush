# Zenhush on Ubuntu/Debian - source install (no binaries)

For people who just want the app: this installs Zenhush from source in a
few minutes. Nothing is compiled; the only download is Qt for Python from
PyPI (Qt's own distribution).

## What you do

1. Extract `zenhush-setup-ubuntu.tar.gz` (double-click it in Files).
2. Open the extracted folder, right-click > "Open in Terminal".
3. Run: `bash install.sh`

## What the installer changes on your machine

- System packages (only if missing, asks your password via sudo):
  `libxcb-cursor0`, `python3-venv` (and `python3`/`python3-pip` only if
  python3 itself is absent). Installed with apt, removable with apt.
- `~/.local/share/zenhush/` - the app source plus its private Python
  environment (venv). No system-wide pip packages are installed or changed.
- `~/.local/share/applications/zenhush.desktop` - the app-grid entry.
- Nothing else. No git repository is cloned, no existing repo or branch is
  touched, no autostart or background service is added.

## Requirements

- Ubuntu/Debian with apt; Python 3.9+ (present on Ubuntu 20.04+).
- Internet once, for the ~245 MB Qt for Python download (pinned:
  PySide6-Essentials==6.11.3, PySide6-Addons==6.11.3).
- A system tray / app-indicator host for the tray icon. Without one the
  app falls back to a small normal window - everything still works.
- Verified on an Ubuntu-22.04-like X11 + PulseAudio setup; other OS
  versions and Wayland sessions are untested.

## Uninstall

```
rm -rf ~/.local/share/zenhush ~/.local/share/applications/zenhush.desktop
```

Optionally also `sudo apt remove libxcb-cursor0` if nothing else needs it.

## This exact build

The installer and app source here are commit 0b6b884 on branch
`zenhush-build` (PR #1). The installer was run end-to-end in a clean
home directory during verification: desktop entry passes
desktop-file-validate, the installed app launches and plays (PCM output
captured, RMS 0.052).
