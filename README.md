# Zenhush

**A lightweight, fast, background focus audio machine for your top bar or notch.**

Zenhush lives quietly in your system tray / menu bar (or collapses into a sleek top-center Notch Pill), synthesizes endless stereo focus soundscapes on-device, lets you lock your favorite local audio files as your default audio, and opens instantly with global keyboard shortcuts.

---

## Features

- **8 Endless Stereo Soundscapes** (100% procedural, zero network):
  - **Brown Noise** — deep and warm
  - **Pink Noise** — soft and balanced
  - **White Noise** — bright and even
  - **Rain** — steady rainfall with droplet textures
  - **Ocean** — slow stereo rolling waves
  - **Fireplace** — warm hearth crackle & ember rumble
  - **Night Forest** — gentle wind & nocturnal shimmer
  - **Deep Focus** — warm harmonic ambient drone pad
- **Atmosphere Layering**: Blend rain, fireplace, ocean, brown noise, or forest breeze underneath *any* soundscape or local music track.
- **Local Audio Library & Lock as Default**:
  - Drag & drop or add `.wav`, `.mp3`, `.flac`, `.ogg`, `.m4a` files.
  - Streaming background decoder (< 1 MB RAM, instant start, unlimited track duration, optional smooth loop crossfade).
  - **Lock as Default** (`🔒`) pins a local audio track as Zenhush's permanent startup default.
  - Independent **Tone** modes (`Studio`, `Warm`, `Custom EQ`) so local music stays crisp and unmuffled by default.
- **Top Bar, System Tray & Notch Pill Mode**:
  - Runs as a native background top-bar / menu-bar app (Linux StatusNotifier via `ksni`, macOS `Accessory` menu-bar agent, Windows tray).
  - **Notch Pill Mode** collapses into a slim top-center capsule (`286×42px`) with live mini EQ, sound title, play/pause, and scroll-wheel volume control.
  - **Pin Window**, **Top Notch**, **Top Bar**, or **Draggable** placement.
- **Instant Volume, Timers & Shortcuts**:
  - Always-accessible Volume slider, Mute toggle (`M`), and scroll-wheel volume control.
  - Focus / Sleep timer (`15m`, `25m`, `30m`, `45m`, `60m`, `90m`) with gentle 3-second fade-out.
  - Single-instance IPC (`zenhush --toggle`, `--play-pause`, `--vol-up`, `--notch`, `--background`), in-process global hotkeys (X11/macOS/Windows), and 1-click GNOME Wayland shortcut binding.

## Install & Run on Linux

### 1. One-step install from source

```sh
git clone https://github.com/nishachay/zenhush.git
cd zenhush
./install.sh
```

This builds the native release binary (~8.7 MB) and runs `zenhush --install`, which:
- Installs the binary to `~/.local/bin/zenhush`
- Installs the app launcher icon and entry (`~/.local/share/applications/zenhush.desktop`)

*(To uninstall later: `./uninstall.sh` or `zenhush --uninstall`.)*

### 2. Run & Pin to Your Top Bar / Notch

```sh
zenhush --background   # Starts silently in your Linux top bar (StatusNotifier tray)
zenhush --toggle       # Opens / hides the popup from anywhere
```

- **Ubuntu GNOME**: Ensure `ubuntu-appindicators` is enabled (enabled by default on Ubuntu) to see Zenhush in the top bar. Left-click the tray icon to toggle Zenhush, or right-click for playback, sound, volume, timer, Notch Pill, and Pin controls.
- **Notch Pill Mode**: Click the minimize-to-pill button in the top right (or press `N`) to collapse Zenhush into a slim top-center capsule with live EQ, play/pause, and scroll-wheel volume.
- **Global Shortcut on Wayland / X11**: Open the **Bar & Keys** tab (`S`) and click **Bind GNOME Shortcut** (registers `Alt+Shift+Z` to toggle Zenhush and `Alt+Shift+Space` to play/pause via `gsettings`), or bind `zenhush --toggle` in your desktop environment's shortcut settings.
- **Lock Local Audio as Default**: Drag & drop (or add) any `.wav`, `.mp3`, `.flac`, `.ogg`, or `.m4a` file and click the **Lock** icon (`L` or `Add & Lock Default`). Every launch will open straight to your locked track.

### Build manually

```sh
cargo test
cargo build --release
./target/release/zenhush
```

## CLI & IPC Controls

When Zenhush is running in the background, CLI calls wake or control the active instance in milliseconds:

```sh
zenhush --background   # Start silently in the top bar / tray
zenhush --toggle       # Toggle popup visibility
zenhush --play-pause   # Play / Pause
zenhush --next         # Next sound
zenhush --prev         # Previous sound
zenhush --vol-up       # Volume +5%
zenhush --vol-down     # Volume -5%
zenhush --mute         # Mute / Unmute
zenhush --notch        # Toggle Notch Pill mode
zenhush --quit         # Quit background instance
```

## In-App Keyboard Shortcuts

| Key | Action |
|---|---|
| `Space` / `K` | Play / Pause |
| `←` / `→` | Previous / Next sound |
| `↑` / `↓` | Volume +5% / -5% (`Shift` for ±1%) |
| `M` | Mute / Unmute |
| `R` | Restart / Fresh noise seed |
| `L` | Lock / Unlock active local file as default |
| `N` | Toggle Notch Pill / Full Card |
| `P` | Pin / Unpin popup window |
| `T` | Cycle Focus / Sleep timer |
| `S` / `,` | Toggle drawer (`Local Audio`, `Mix & Timer`, `Bar & Keys`) |
| `1`–`9` | Jump directly to a sound or local file |
| `Esc` | Close drawer or hide to top bar |
