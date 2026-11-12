# PyInstaller spec for Zenhush. One-directory layout so the bundled LGPL
# libraries (Qt, FFmpeg) are directly replaceable files under _internal/.
# Linux build tested on Ubuntu 22.04; Windows/macOS binaries need their OS
# runners (see build.yml). Build per OS on that OS's runner:
# pyinstaller packaging/zenhush.spec -> dist/dither/ directory, windowed.

import os

ROOT = os.path.dirname(os.path.abspath(SPEC))
if not os.path.isdir(os.path.join(ROOT, 'zenhush')):
    ROOT = os.path.dirname(ROOT)  # spec lives in packaging/

a = Analysis(
    [os.path.join(ROOT, 'run_zenhush.py')],
    pathex=[ROOT],
    binaries=[],
    datas=[(os.path.join(ROOT, 'zenhush/assets'), 'zenhush/assets'),
          (os.path.join(ROOT, 'packaging/licenses'), 'licenses'),
          (os.path.join(ROOT, 'packaging/THIRD_PARTY_NOTICES.md'), 'THIRD_PARTY_NOTICES.md')],
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
    [],
    exclude_binaries=True,
    name='zenhush',
    debug=False,
    strip=False,
    upx=False,
    console=False,
    icon=os.path.join(ROOT, 'zenhush/assets/icon.png'),
)
coll = COLLECT(
    exe,
    a.binaries,
    a.datas,
    strip=False,
    upx=False,
    name='zenhush',
)
