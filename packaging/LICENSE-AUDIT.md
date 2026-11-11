# License and distribution audit (research, not legal advice)

Prepared 12 Nov 2026, before any public distribution. Status: audit only;
no distribution has happened.

## What dither ships

### Own code and assets
- `dither_app/`, `run_dither.py`, `tests/`, `packaging/` - written for this
  project by Instinct for Nishachay. **No LICENSE file chosen yet.** A
  license must be picked before any public release; until then the code is
  all-rights-reserved by default.
- All audio is synthesized on the device by `noise_gen.py` (pure Python
  standard library). No recordings, samples, or third-party audio content
  are bundled, so there are no sample or field-recording licenses to clear.
- Icon and tray PNGs were generated locally for this project (Pillow), no
  third-party artwork.

### Bundled third-party components (in the PyInstaller binary)
- **Qt 6.12 via PySide6 (LGPL v3)**. Obligations when distributing:
  include the LGPL license text and a Qt attribution notice, state that Qt
  is used and where to get it, and keep Qt dynamically linked (it is, in
  the PyInstaller layout) or otherwise allow relinking. Qt itself is
  unmodified, so no Qt source offer beyond pointing at upstream.
- **FFmpeg 7.1.5 (LGPL v2.1+)**, used by Qt Multimedia's playback backend.
  Same LGPL notice obligations; FFmpeg is unmodified and dynamically
  loaded by Qt.
- **Python 3.10 runtime (PSF license)**, permissive; include the PSF notice.
- **PyInstaller 6 bootloader** (GPL with a runtime exception that permits
  freezing unmodified programs); no obligation beyond not modifying the
  bootloader.

### System components the binary needs at runtime (not bundled)
- X11 or Wayland desktop with a system tray host (on Linux, a
  StatusNotifier host such as GNOME's AppIndicator extension or KDE's tray)
- PulseAudio or PipeWire (pulse API) for audio output
- `libxcb-cursor0` on X11 (Debian/Ubuntu package name); the app does not
  start on X11 without it

## What the test package ships (shipped 12 Nov 2026)
- One-directory PyInstaller build: Qt/FFmpeg shared libraries are
  directly replaceable files in dither/_internal/.
- Full license texts (GPL-3.0, LGPL-3.0, LGPL-2.1, PSF, PyInstaller)
  and THIRD_PARTY_NOTICES.md. Upstream corresponding-source URLs were
  verified live; a written offer is NOT issued with this private test
  package - corresponding-source arrangements are item 2 in the gaps
  below and stay open until a release decision.
- dither's own source included under source/ (private share, no public
  license selected yet).

## Gaps to close before public release
1. Pick a LICENSE for the project's own code.
2. Trademark clearance for the name "dither" (not done - see brand File).
3. Windows/macOS binaries are not built; no claims until CI or real
   machines run the test suites there.

## Version pin note (12 Nov 2026)
PySide6 is pinned to 6.11.3 (Qt 6.11.3, FFmpeg 7.1.5). The 6.12.0 wheels
crash on Python < 3.12 (singleton refcount corruption, none_dealloc aborts;
Qt PYSIDE-3424/PYSIDE-3474). Observed live: repeated aborts during custom
painting under 6.12.0; the same workload runs clean on 6.11.3.
