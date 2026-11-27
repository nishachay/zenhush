# Zenhush

**A minimal, fast focus audio app that lives right inside your top bar.**

Click the icon in your GNOME top bar to play/pause, pick a focus soundscape, drag volume (or scroll right on the top-bar icon), set a sleep timer, or lock your favorite local audio file as your startup default — with zero floating window clutter and ~0% idle CPU.

---

## Features

- **Lives in Your Top Bar (Native GNOME Popover)**:
  - No separate floating window getting in the way: clicking Zenhush in the GNOME top panel drops open the clean native popover directly below the icon.
  - **Middle-click** the top-bar icon to Play/Pause instantly; **scroll wheel** right on the icon to adjust volume.
  - Zero OpenGL/GPU window overhead when using the top-bar popover (keeps your laptop cool and silent).
  - Includes a clean lightweight fallback popup card for macOS, Windows, and non-GNOME Linux desktops.
- **8 Endless Stereo Soundscapes** (100% procedural on-device, zero network):
  - **Brown**, **Pink**, **White**, **Rain**, **Ocean**, **Fire**, **Forest**, **Focus**.
- **Your Audio File & Lock as Default**:
  - Choose any `.wav`, `.mp3`, `.flac`, `.ogg`, or `.m4a` file (streaming decoder, < 1 MB RAM, instant start, seamless loop crossfade).
  - One click on **Lock** pins your track as Zenhush's startup default.
- **Sleep Timer**:
  - One-tap cycle (`Off → 25m → 45m → 60m → 90m`) with gentle 3-second fade-out.
- **Hardware Media Keys & MPRIS2**:
  - Keyboard media keys (`Play/Pause`, `Next`, `Prev`) and GNOME's quick-settings media card control Zenhush automatically.

## Install & Run

### Download Binary (Recommended)

1. Download the latest release binary from [Releases](https://github.com/nishachay/zenhush/releases/latest) (`zenhush-linux-x86_64`, `zenhush-macos`, or `zenhush-windows-x86_64.exe`).
2. On Linux: mark executable (`chmod +x zenhush-linux-x86_64`) and run:
   ```sh
   ./zenhush-linux-x86_64 --install
   ```
   This installs Zenhush to `~/.local/bin/zenhush` and adds it to your app launcher.
3. Launch **Zenhush** from your app menu. On Ubuntu/GNOME, Zenhush automatically installs its Top-Bar Popover extension (`zenhush@zenhush.app`) — log out and back in once if prompted so it snaps directly into the top panel.

### Build from Source

```sh
git clone https://github.com/nishachay/zenhush.git
cd zenhush
cargo test --locked
cargo build --release --locked
./target/release/zenhush --install
```

*(To uninstall later: `zenhush --uninstall && zenhush --uninstall-gnome-pill`.)*

## CLI Controls

```sh
zenhush --background   # Start silently in the top bar (~15 MB RSS, 0% idle CPU)
zenhush --toggle       # Toggle top-bar popover / popup
zenhush --play-pause   # Play / Pause
zenhush --next         # Next sound
zenhush --prev         # Previous sound
zenhush --vol-up       # Volume +5%
zenhush --vol-down     # Volume -5%
zenhush --mute         # Mute / Unmute
zenhush --quit         # Quit running instance
```
