"""Standalone entry point for frozen builds (PyInstaller).

Uses an absolute import so it works both as a plain script and frozen;
`python -m dither_app` stays the source-run entry.
"""
import sys

from dither_app.app import main

sys.exit(main())
