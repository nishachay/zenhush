# Licensing and source-distribution assessment (pre-release draft)

Status: engineering checklist for the first distributable build. Review
before any public release. Artifact naming is provisional ("dither") until
the user picks the final name; the name appears in notices then.

## Components and obligations

| Component | Version | License | How it ships | Obligations |
|---|---|---|---|---|
| Qt / PySide6 | 6.11.3 | LGPL v3 | Dynamically linked, bundled by PyInstaller | Notice + license text (done: packaging/licenses/LGPL-3.0.txt); Qt source availability: point to https://download.qt.io/official_releases/qt/; users must be able to swap the library - for a Python app that means documenting `pip install PySide6==6.11.3` replacement (see below) |
| FFmpeg (Qt Multimedia backend) | 7.1.5 | LGPL v2.1+ | Shared libraries inside PySide6 wheels, loaded by Qt | Notice + license text (done: LGPL-2.1.txt); source availability: https://ffmpeg.org/download.html and Qt's mirrored sources |
| Schibsted Grotesk | bundled | SIL OFL 1.1 | Font files in dither_app/assets/fonts/ | OFL.txt already bundled with the font (done) |
| Lucide icons | bundled | ISC | SVG files in dither_app/assets/icons/ | LICENSE-lucide already bundled (done) |
| dither code + noise synthesis | this repo | TBD by owner | Source + binary | Owner picks the project license before release |

## Relinking / replacement note (to ship with the app)

The app is Python: no static linking anywhere. Replacing Qt/PySide6 or
FFmpeg means installing the public wheels (`pip install PySide6==6.11.3`)
and running from source (`python -m dither_app`), or swapping the shared
libraries inside the frozen bundle. This satisfies the LGPL relink
requirement without shipping object files.

## What we deliberately do NOT do

- No multi-year written source offer. Qt and FFmpeg sources are publicly
  available at the URLs above; notices point there.
- No license change for bundled assets; OFL and ISC files ship unmodified.

## Open items before first release

- [ ] Owner picks the project's own license (repo has none yet).
- [ ] Insert the final app name into all notices once decided.
- [ ] Include packaging/licenses/ inside the frozen bundle (add to dither.spec datas) and a notices file at the bundle root.
- [ ] Windows/macOS builds: same obligations, bundle layout differs.
- [ ] This is an engineering checklist, not legal advice.
