# ZenHush — Native Rust Architecture

Zenhush is a lightweight, self-contained native Rust binary with a split **Background Core + On-Demand UI** architecture:

- **Background Core (`src/daemon.rs`, `src/ipc.rs`, `src/bridge.rs`)**: Runs `AudioEngine`, `Tray`, `MprisServer`, `ShortcutManager`, and `BridgeServer` with zero OpenGL/GPU allocations (~15 MB RSS, 0% idle CPU when paused).
- **On-Demand UI (`src/app.rs`, `src/theme.rs`, `src/icons.rs`)**: `eframe` / `egui` (`glow` backend only) connects over local socket (`BridgeClient`) when opened and exits cleanly on hide/close to reclaim GPU/Mesa RAM.
- **Audio (`src/audio.rs`, `src/synth.rs`, `src/filemode.rs`)**: `cpal` stereo output at the device's native sample rate, procedural stereo synthesis for 8 focus soundscapes + atmosphere layering, and a non-blocking `symphonia` + `rtrb` streaming decoder for local audio files with overlap-add loop crossfading.
- **Desktop Integration (`src/tray.rs`, `src/mpris.rs`, `src/shortcuts.rs`, `src/gnome_ext.rs`)**:
  - Linux: pure-Rust D-Bus StatusNotifier (`ksni`, no GTK) with middle-click play/pause & scroll volume, MPRIS2 D-Bus (`zbus`), single-instance Unix socket IPC, GNOME `gsettings` shortcut binding, and an optional GNOME 45–50 Top-Bar Pill extension.
  - macOS / Windows: `tray-icon` + `global-hotkey` (`NSApplicationActivationPolicy::Accessory` on macOS so no Dock icon appears).
- **Persistence (`src/settings.rs`)**: JSON settings and local track library (`~/.config/zenhush/settings.json` on Linux), including `locked_default_file` for locking a local audio track as the startup default.

## Build & Verify

```sh
cargo test
cargo build --release
./target/release/zenhush
```
