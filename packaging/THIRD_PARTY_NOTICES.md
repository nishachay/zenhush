# Third-party notices for the dither Linux binary

This file ships with the dither test binary. The binary is a PyInstaller
one-file bundle that contains the third-party components listed below.
Full license texts are in `licenses/` next to this file.

## dither's own code and content
- All application code (`dither_app/`, `run_dither.py`), tests and
  packaging scripts were written for this project. No public license has
  been chosen yet; until one is, the code is all-rights-reserved.
- All sound is synthesized at runtime by `noise_gen.py` (Python standard
  library only). No audio recordings or samples are bundled.
- Icon/tray PNGs were generated for this project; no third-party artwork.

## Qt 6.12.0 - GNU LGPL v3
- Used via the PySide6 6.12.0 Python wheels (`PySide6-Essentials`,
  `PySide6-Addons`). Qt is unmodified and linked dynamically (shared
  libraries inside the bundle, not static).
- Qt is Copyright The Qt Company Ltd. and other contributors.
- Upstream: https://www.qt.io/ - source for this release:
  https://download.qt.io/official_releases/qt/6.12/6.12.0/
- License text: licenses/LGPL-3.0.txt (LGPL v3 applies the terms of
  GPL v3, licenses/GPL-3.0.txt, plus additional permissions).

## FFmpeg 9.0.1 - GNU LGPL v2.1 or later
- Used by Qt Multimedia's playback backend. Unmodified; loaded as shared
  libraries (libavcodec 63.1.101, libavformat 63.1.101, libavutil 61.1.101,
  libswresample 7.1.101, libswscale 10.1.101).
- Upstream and source: https://ffmpeg.org/ (release tag n9.0.1).
- License text: licenses/LGPL-2.1.txt.

## Python 3.10 runtime - PSF License
- License text: licenses/PSF-Python-3.10-LICENSE.txt.

## PyInstaller 6.22.3 bootloader - GPL v2 with bootloader exception
- The bootloader's license permits frozen programs to be distributed
  under their own terms. Text: licenses/PyInstaller-COPYING.txt.

## Replacing the LGPL libraries
LGPL gives recipients the right to use modified versions of Qt and
FFmpeg with this application. Two practical routes:
1. From source (simplest): install your own Qt/FFmpeg build
   (`pip install PySide6==6.12.0` or a system/custom Qt) and run
   `python -m dither_app` from the project source, or rebuild the frozen
   binary with `pyinstaller packaging/dither.spec`. The source build
   dynamically links whatever Qt/FFmpeg you installed.
2. Note on this one-file build: the bundled `.so` files are packed inside
   the single executable and extracted to a temporary directory at
   runtime, so in-place replacement is impractical. A future public
   release should use the one-directory PyInstaller layout so the Qt and
   FFmpeg shared libraries sit in `_internal/` as directly replaceable
   files.
