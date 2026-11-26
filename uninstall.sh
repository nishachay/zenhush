#!/usr/bin/env bash
set -euo pipefail

if command -v zenhush >/dev/null 2>&1; then
    zenhush --quit 2>/dev/null || true
    zenhush --uninstall
elif [ -x "$HOME/.local/bin/zenhush" ]; then
    "$HOME/.local/bin/zenhush" --quit 2>/dev/null || true
    "$HOME/.local/bin/zenhush" --uninstall
else
    rm -f "$HOME/.local/bin/zenhush" \
          "$HOME/.local/share/applications/zenhush.desktop" \
          "$HOME/.local/share/icons/hicolor/128x128/apps/zenhush.png" \
          "$HOME/.config/autostart/zenhush.desktop"
    echo "Zenhush user files removed."
fi
