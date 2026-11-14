# ZenHush — Rust version

System-tray noise machine: live brown / white / pink / rain / ocean synth
plus file loop, sleep timer, tray popup. This Rust version is the current
app and supersedes the Python (PySide6) version; the Python files remain
in this repo for reference only.

Stack: cpal (audio), egui / eframe (popup UI), ksni (tray, no GTK),
symphonia (file decoding: wav, mp3, flac, ogg, m4a), resvg (Lucide SVG
icons). License: MIT.

## Build and run

```sh
cargo build --release   # binary: target/release/zenhush (~9 MB)
./target/release/zenhush
cargo test --release    # synth tests
```

Linux runtime needs: an audio server (ALSA / PipeWire / PulseAudio) and
`libxkbcommon-x11` (the app will not start without it).

## Fonts — required, not in this repo

The UI font is Schibsted Grotesk (OFL license). The three TTFs are binary
and cannot be pushed through this tool, so they are NOT in the repo. The
build fails without them (`include_bytes!` in `src/main.rs`).

Get them before building:

1. Open https://fonts.google.com/specimen/Schibsted+Grotesk and use
   "Download family" (OFL, free).
2. Unzip and copy these three files into `assets/fonts/` with exactly
   these names:
   - `SchibstedGrotesk.ttf` (Regular)
   - `SchibstedGrotesk-Bold.ttf`
   - `SchibstedGrotesk-SemiBold.ttf`

## Icons

- `assets/icons/*.svg` (Lucide) are in the repo; they are text.
- `assets/tray.png` is NOT in the repo (binary). The tray shows no icon
  without it; the app still runs.
- `assets/icon.png` is not used by the code.

## Status / known issues (2026-11-12)

- Builds; synth tests pass; playback works.
- Visual alignment is unresolved: the settings-screen fixes (52px label
  column, full-width sliders, right-edge values) were made but never
  approved by the owner.
- RAM is NOT proven lighter than the Python app (Xvfb idle: Rust
  RSS ~122 MB vs Python ~91 MB).
- In a VM without audio the status shows "No audio device".

UI screenshots of the current build exist locally but are not pushed
(binary PNGs).
