# Third-party notices for the dither Linux binary

This file ships with the dither test package. The application directory
`dither/` is a PyInstaller one-directory build: the executable
`dither/dither` loads shared libraries from `dither/_internal/`, where
every library sits as a separate, directly replaceable file. Full
license texts are in `licenses/`; corresponding-source details and the
formal written offer are in `WRITTEN_OFFER.txt`.

## dither's own code and content
- All application code (`dither_app/`, `run_dither.py`), tests and
  packaging scripts were written for this project and are included in
  this package under `source/`. No public license has been chosen yet;
  until one is, the code is all-rights-reserved and shared privately
  with this package's recipient only.
- All sound is synthesized at runtime by `noise_gen.py` (Python standard
  library only). No audio recordings or samples are bundled.
- Icon/tray PNGs were generated for this project; no third-party artwork.

## Qt 6.12.0 - GNU LGPL v3
- Used via the PySide6 6.12.0 Python wheels. Qt is unmodified and
  dynamically linked: `dither/_internal/libQt6*.so.6`.
- Qt is Copyright The Qt Company Ltd. and other contributors.
- License texts: licenses/LGPL-3.0.txt plus licenses/GPL-3.0.txt
  (LGPL v3 applies GPL v3 with additional permissions).
- Corresponding source: see WRITTEN_OFFER.txt (upstream Qt source URL
  verified live 12 Nov 2026).

## FFmpeg 9.0.1 - GNU LGPL v2.1 or later
- Used by Qt Multimedia's playback backend. Unmodified shared libraries:
  `dither/_internal/libavcodec.so.63` (63.1.101), `libavformat.so.63`,
  `libavutil.so.61`, `libswresample.so.7`, `libswscale.so.10`.
  Version confirmed from the "FFmpeg version 9.0.1" string embedded in
  the bundled libavcodec.
- License text: licenses/LGPL-2.1.txt.
- Corresponding source: see WRITTEN_OFFER.txt (ffmpeg.org release URL
  verified live 12 Nov 2026).

## Python 3.10.12 runtime - PSF License
- License text: licenses/PSF-Python-3.10-LICENSE.txt.

## PyInstaller 6.22.3 bootloader - GPL v2 with bootloader exception
- The bootloader license permits frozen programs to be distributed
  under their own terms. Text: licenses/PyInstaller-COPYING.txt.

## Replacing the LGPL libraries
Because this is a one-directory build, you can replace Qt or FFmpeg
directly: drop your own builds of the `libQt6*.so.6` or `libav*.so` /
`libsw*.so` files into `dither/_internal/` (keeping the sonames) and
run `dither/dither`. Alternatively, install your own Qt/FFmpeg
(`pip install PySide6==6.12.0` or a custom build) and run the app from
`source/` with `python3 run_dither.py`; the source run links whatever
Qt/FFmpeg you installed.
