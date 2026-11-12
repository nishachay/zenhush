#!/usr/bin/env bash
# Zenhush - one-step installer for Ubuntu/Debian (source build, no binaries included).
# Double-click won't run this; open a Terminal and run:
#   bash install.sh
set -euo pipefail
cd "$(dirname "$0")"

echo "Zenhush setup - this will:"
echo "  1. install system packages it needs (libxcb-cursor0, python3 venv) - may ask your password"
echo "  2. put Zenhush in ~/.local/share/zenhush"
echo "  3. add it to your app grid like any other app"
echo ""

if ! command -v python3 >/dev/null; then
  echo "python3 not found - installing..."
  sudo apt-get update && sudo apt-get install -y python3 python3-venv python3-pip
fi

PYVER=$(python3 -c 'import sys; print(f"{sys.version_info.major}.{sys.version_info.minor}")')
python3 -c 'import sys; sys.exit(0 if sys.version_info >= (3, 9) else 1)' || {
  echo "Zenhush needs Python 3.9 or newer; found $PYVER. On Ubuntu 20.04+ this is already fine - please update your system."; exit 1; }

MISSING=""
dpkg -s libxcb-cursor0 >/dev/null 2>&1 || MISSING="$MISSING libxcb-cursor0"
python3 -m venv --help >/dev/null 2>&1 || MISSING="$MISSING python3-venv"
if [ -n "$MISSING" ] && [ "${ZENHUSH_ASSUME_DEPS:-0}" != "1" ]; then
  echo "Installing missing system packages:$MISSING"
  sudo apt-get update && sudo apt-get install -y $MISSING
fi

APP_DIR="$HOME/.local/share/zenhush"
mkdir -p "$APP_DIR"
echo "Copying Zenhush to $APP_DIR ..."
rm -rf "$APP_DIR/zenhush" "$APP_DIR/venv"
cp -r src/zenhush "$APP_DIR/zenhush"

echo "Creating its Python environment (downloads Qt for Python ~245 MB once) ..."
python3 -m venv "$APP_DIR/venv"
"$APP_DIR/venv/bin/pip" install --upgrade pip
"$APP_DIR/venv/bin/pip" install PySide6-Essentials==6.11.3 PySide6-Addons==6.11.3

mkdir -p "$HOME/.local/share/applications"
cat > "$HOME/.local/share/applications/zenhush.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Zenhush
Comment=Noise for focus and rest
Exec=$APP_DIR/venv/bin/python -m zenhush
Path=$APP_DIR
Icon=$APP_DIR/zenhush/assets/icon.png
Terminal=false
Categories=AudioVideo;Audio;Player;
Keywords=noise;focus;sleep;brown;white;pink;
DESKTOP
update-desktop-database "$HOME/.local/share/applications" >/dev/null 2>&1 || true

echo ""
echo "Done. Zenhush is now in your app grid (search 'Zenhush')."
echo "First run: click its tray icon, pick a sound, press play."
