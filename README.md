# dither

A tiny noise machine for your top bar.

dither plays locally synthesized noise (brown, white, pink, rain, ocean)
from a system-tray icon with a compact popup: play/pause, sound picker,
volume, and a sleep timer. All audio is generated on the device by
`noise_gen.py` - nothing is downloaded and no network access is used.

Cross-platform desktop app built with Python and PySide6 (Qt):
`QSystemTrayIcon` for the tray (StatusNotifier on Linux) and
QtMultimedia for gapless looping playback.

## Run from source

```
pip install PySide6 PySide6-Addons
python -m dither_app
```

Requires a desktop session with a system tray. Without a tray host the
app falls back to a small normal window.

## Tests

```
python tests/test_noise.py   # audio engine checks (spectral shape, loop seams)
python tests/test_app.py     # UI checks (offscreen Qt, no display needed)
```

## Status

Working name approved, trademark and domain checks still open before any
public release. Packaged binaries are not built yet.
