#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"

echo "==> Installing Zenhush for current user..."

if [ -f "$HOME/.cargo/env" ]; then
    # shellcheck source=/dev/null
    . "$HOME/.cargo/env"
fi

if [ ! -x "$ROOT/target/release/zenhush" ]; then
    if ! command -v cargo >/dev/null 2>&1; then
        echo "Rust (cargo) is required to build from source. Install from https://rustup.rs"
        exit 1
    fi
    echo "==> Building release binary..."
    cargo build --release
fi

"$ROOT/target/release/zenhush" --install

if [[ ":${PATH}:" != *":$HOME/.local/bin:"* ]]; then
    echo "Note: add ~/.local/bin to your PATH if 'zenhush' is not found in your shell."
fi

echo "==> Done! Launch Zenhush from your app menu, or run:"
echo "    zenhush --background   # start in top bar / tray"
echo "    zenhush --toggle       # show/hide popup"
