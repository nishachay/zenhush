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
- **FFmpeg 9.0.1 (LGPL v2.1+)**, used by Qt Multimedia's playback backend.
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
- Full license texts (GPL-3.0, LGPL-3.0, LGPL-2.1, PSF, PyInstaller),
  THIRD_PARTY_NOTICES.md, and WRITTEN_OFFER.txt with upstream
  corresponding-source URLs verified live, plus a 3-year written offer.
- dither's own source included under source/ (private share, no public
  license selected yet).

## Gaps to close before public release
1. Pick a LICENSE for the project's own code.
2. Trademark clearance for the name "dither" (not done - see brand File).
3. Windows/macOS binaries are not built; no claims until CI or real
   machines run the test suites there.
