# Zenhush

**A minimal, fast focus audio app that lives right inside your top bar.**

Click Zenhush in your GNOME top bar to play/pause, pick a focus soundscape, adjust volume, set a sleep timer, or pin a local audio track as your startup default — with zero floating window clutter and ~0% idle CPU.

---

## Features

- **Lives in Your Top Bar (Native GNOME Popover)**:
  - No separate floating window getting in the way: clicking Zenhush in the GNOME top bar drops open the native popover directly below the pill.
  - **Middle-click** the top-bar pill to Play/Pause instantly.
  - Zero OpenGL/GPU window overhead when using the GNOME top-bar popover.
  - Includes a crisp content-fit floating card (`336 px` wide) on macOS, Windows, non-GNOME Linux desktops, and first-run GNOME before session activation.
- **8 Endless Stereo Soundscapes** (100% procedural on-device, zero network):
  - **Brown**, **Pink**, **White**, **Rain**, **Ocean**, **Fire**, **Forest**, **Focus**.
- **Your Audio File & Startup Default Lock**:
  - Choose or drop any `.wav`, `.mp3`, `.flac`, `.ogg`, or `.m4a` file (streaming decoder, seamless loop crossfade, automatically remembered across launches).
  - One click on **Default** locks it so Zenhush always starts with your chosen track even after playing built-in sounds.
- **Sleep Timer**:
  - One-tap cycle (`Off → 25m → 45m → 60m → 90m`) with gentle 3-second fade-out.
- **Hardware Media Keys & MPRIS2**:
  - Keyboard media keys (`Play/Pause`, `Next`, `Prev`) and GNOME's quick-settings media card control Zenhush automatically.

## Install & Run

### Download Release (Recommended)

1. Download the latest binaries (or Debian/Ubuntu `.deb` package) and verify integrity with `SHA256SUMS.txt` from [Releases](https://github.com/nishachay/zenhush/releases/latest).
2. **Ubuntu / Debian (`.deb`)**:
   ```sh
   sudo apt install ./zenhush_*_amd64.deb
   ```
3. **Standalone Linux binary**:
   ```sh
   sha256sum -c SHA256SUMS.txt --ignore-missing
   chmod +x zenhush-linux-x86_64 && ./zenhush-linux-x86_64 --install
   ```
   This installs Zenhush to `~/.local/bin/zenhush`, creates the `.desktop` entry, and refreshes the XDG icon cache. On GNOME Wayland, log out & back in once if prompted so the native top-bar popover activates.

### Build from Source

```sh
git clone https://github.com/nishachay/zenhush.git
cd zenhush
cargo test --locked
cargo build --release --locked
./target/release/zenhush --install
```

*(To uninstall completely: `zenhush --uninstall`.)*

## CLI Controls

```sh
zenhush --background   # Start silently in the top bar / tray (~15 MB RAM)
zenhush --toggle       # Toggle top-bar popover / card
zenhush --play-pause   # Play / Pause
zenhush --next         # Next sound
zenhush --prev         # Previous sound
zenhush --vol-up       # Volume +5%
zenhush --vol-down     # Volume -5%
zenhush --mute         # Mute / Unmute
zenhush --version      # Print version
zenhush --quit         # Quit running instance
```
