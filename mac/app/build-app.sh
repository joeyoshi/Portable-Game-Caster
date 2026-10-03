#!/bin/bash

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
MAC_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PROJECT_DIR="$(cd "$MAC_DIR/.." && pwd)"

LAUNCHER_DIR="$MAC_DIR/launcher"
APP_DIR="$SCRIPT_DIR/dist/Portable Game Caster.app"

echo "Building Portable Game Caster macOS launcher..."

cd "$LAUNCHER_DIR"
cargo build --release

echo "Creating app bundle..."

rm -rf "$APP_DIR"

mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

cp "$SCRIPT_DIR/Info.plist" \
   "$APP_DIR/Contents/Info.plist"

cp "$LAUNCHER_DIR/target/release/pgc-launcher-macos" \
   "$APP_DIR/Contents/MacOS/pgc-launcher-macos"

chmod +x "$APP_DIR/Contents/MacOS/pgc-launcher-macos"

echo
echo "Built:"
echo "$APP_DIR"