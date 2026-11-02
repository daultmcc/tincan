#!/bin/sh
# Build TinCan and install it for the current user (or under $PREFIX).
set -eu
PREFIX="${PREFIX:-$HOME/.local}"
cd "$(dirname "$0")"
cargo build --release
install -Dm755 target/release/tincan "$PREFIX/bin/tincan"
install -Dm644 data/tincan.desktop "$PREFIX/share/applications/tincan.desktop"
install -Dm644 crates/tincan/icons/tincan.svg "$PREFIX/share/icons/hicolor/scalable/apps/tincan.svg"
command -v update-desktop-database >/dev/null && update-desktop-database "$PREFIX/share/applications" || true
echo "Installed to $PREFIX. Make sure $PREFIX/bin is on your PATH."
