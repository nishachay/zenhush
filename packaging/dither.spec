# PyInstaller spec for dither. TEMPLATE: Linux binary built and smoke-tested
# locally; Windows/macOS binaries need their OS runners (see build.yml).
# Build per OS on that OS's runner: pyinstaller packaging/dither.spec
# Produces dist/dither (or dither.exe), one-file, windowed (no console).

import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(SPEC)))

a = Analysis(
    [os.path.join(ROOT, 'run_dither.py')],
    pathex=[ROOT],
    binaries=[],
    datas=[(os.path.join(ROOT, 'dither_app/assets'), 'dither_app/assets')],
    hiddenimports=[],
    hookspath=[],
    runtime_hooks=[],
    excludes=['numpy', 'pytest'],
    noarchive=False,
)
pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    name='dither',
    debug=False,
    strip=False,
    upx=False,
    console=False,
    icon=os.path.join(ROOT, 'dither_app/assets/icon.png'),
)
