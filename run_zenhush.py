"""Standalone entry point for frozen builds (PyInstaller).

Uses an absolute import so it works both as a plain script and frozen;
`python -m zenhush` stays the source-run entry.
"""
import sys

from zenhush.app import main

sys.exit(main())
