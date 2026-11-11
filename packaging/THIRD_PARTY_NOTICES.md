# Third-party notices for the dither Linux binary

This file ships with the dither package. The application directory
`dither/` is a PyInstaller one-directory build: the executable
`dither/dither` loads shared libraries from `dither/_internal/`, where
every library sits as a separate, directly replaceable file. Full
license texts are in `licenses/`.

## dither's own code and content
- All application code (`dither_app/`, `run_dither.py`), tests and
  packaging scripts were written for this project and are included in
  this package under `source/`. No public license has been chosen yet;
  until one is, the code is all-rights-reserved and shared privately
  with this package's recipient only.
- All sound is synthesized at runtime; no audio recordings or samples
  are bundled.
- Icon/tray PNGs were generated for this project; no third-party artwork.

## Qt 6.11.3 - GNU LGPL v3
- Used via the official PySide6 6.11.3 Python wheels, unmodified and
  dynamically linked: `libQt6*.so.6` under `dither/_internal/` and
  `dither/_internal/PySide6/Qt/lib/`.
- Qt is Copyright The Qt Company Ltd. and other contributors.
- License texts: licenses/LGPL-3.0.txt plus licenses/GPL-3.0.txt
  (LGPL v3 applies GPL v3 with additional permissions).
- Corresponding source: Qt builds the PySide6 wheels from the official
  Qt release sources. The complete corresponding source for Qt 6.11.3
  is available at no charge from
  https://download.qt.io/official_releases/qt/6.11/6.11.3/single/
  (directory verified live 12 Nov 2026).

## FFmpeg 7.1.5 - GNU LGPL v2.1 or later
- Used by Qt Multimedia's playback backend. Unmodified shared libraries
  from the PySide6 wheels: `libavcodec.so.61`, `libavformat.so.61`,
  `libavutil.so.59`, `libswresample.so.5`, `libswscale.so.8` under
  `dither/_internal/`. Version confirmed from the
  "FFmpeg version 7.1.5" string embedded in the bundled libavcodec.
- License text: licenses/LGPL-2.1.txt.
- Corresponding source: the complete FFmpeg 7.1.5 source is available
  at no charge from https://ffmpeg.org/releases/ffmpeg-7.1.5.tar.xz
  (file verified live 12 Nov 2026).

## PySide6 / Shiboken 6.11.3 - GNU LGPL v3
- The Python bindings for Qt, unmodified from the official PySide6 6.11.3
  wheels: `libpyside6.abi3.so.6.11`, `libshiboken6.abi3.so.6.11` and the
  Qt*.abi3.so extension modules under `dither/_internal/`.
- License texts: licenses/LGPL-3.0.txt plus licenses/GPL-3.0.txt.
- Corresponding source: the complete PySide6/Shiboken 6.11.3 source
  (pyside-setup, including build scripts) is available at no charge from
  https://download.qt.io/official_releases/QtForPython/pyside6/PySide6-6.11.3-src/
  (directory verified live 12 Nov 2026). Replaceable the same way as Qt:
  drop rebuilt files into `dither/_internal/`, or
  `pip install PySide6==6.11.3` and run from `source/`.

## Python 3.10.12 runtime - PSF License
- License text: licenses/PSF-Python-3.10-LICENSE.txt.

## PyInstaller 6.22.3 bootloader - GPL v2 with bootloader exception
- The bootloader license permits frozen programs to be distributed
  under their own terms. Text: licenses/PyInstaller-COPYING.txt.

## Replacing the LGPL libraries
Because this is a one-directory build, you can replace Qt or FFmpeg
directly: drop your own builds of the `libQt6*.so.6` or `libav*.so` /
`libsw*.so` files into `dither/_internal/` (and
`dither/_internal/PySide6/Qt/lib/`, keeping the sonames) and run
`dither/dither`. Alternatively, install your own Qt/FFmpeg
(`pip install PySide6==6.11.3` or a custom build) and run the app from
`source/` with `python3 run_dither.py`; the source run links whatever
Qt/FFmpeg you installed.

## Schibsted Grotesk (UI font) - SIL Open Font License 1.1
- Source: google/fonts repository, ofl/schibstedgrotesk (project:
  github.com/schibsted/schibsted-grotesk).
- Bundled unmodified at dither_app/assets/fonts/SchibstedGrotesk.ttf
  (variable font, Regular-Black); OFL text beside it at
  dither_app/assets/fonts/OFL.txt. OFL 1.1 permits this bundling.

## Lucide (UI icons) - ISC License
- Source: github.com/lucide-icons/lucide
  (icons: play, pause, rotate-ccw, settings, chevron-left, chevron-right).
- Recolored at render time via currentColor; play/pause additionally
  filled for the solid-button style. No shape edits.
- ISC text bundled at dither_app/assets/icons/LICENSE-lucide.
