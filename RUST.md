# ZenHush — Native Rust Architecture

Zenhush 2.0 is a lightweight, self-contained native Rust binary:

- **Audio (`src/audio.rs`, `src/synth.rs`, `src/filemode.rs`)**: `cpal` stereo output at the device's native sample rate, procedural stereo synthesis for 8 focus soundscapes + atmosphere layering, and a non-blocking `symphonia` + `rtrb` streaming decoder for local audio files with overlap-add loop crossfading.
- **UI (`src/app.rs`, `src/theme.rs`, `src/icons.rs`)**: `eframe` / `egui` (`glow` backend only, no `wgpu`/Vulkan bloat), bundled `Schibsted Grotesk` weights, and `resvg`-rasterized Lucide SVG icons.
- **Background & Shortcuts (`src/tray.rs`, `src/ipc.rs`, `src/shortcuts.rs`)**:
  - Linux: pure-Rust D-Bus StatusNotifier (`ksni`, no GTK) with dynamic amber active-playback badge, plus single-instance Unix socket IPC and GNOME `gsettings` shortcut binding.
  - macOS / Windows: `tray-icon` + `global-hotkey` (`NSApplicationActivationPolicy::Accessory` on macOS so no Dock icon appears).
- **Persistence (`src/settings.rs`)**: JSON settings and local track library (`~/.config/zenhush/settings.json` on Linux), including `locked_default_file` for locking a local audio track as the startup default.

## Build & Verify

```sh
cargo test
cargo build --release
./target/release/zenhush
```
