#!/bin/bash
set -e

echo "🚧 Starting Zlack Build (Unix)..."
npm run tauri build

DIST_DIR="dists"
mkdir -p "$DIST_DIR"

echo "📦 Copying artifacts to $DIST_DIR..."

# macOS DMG
DMG_PATH="src-tauri/target/release/bundle/dmg/*.dmg"
if compgen -G "$DMG_PATH" > /dev/null; then
    cp $DMG_PATH "$DIST_DIR/"
    echo "  ✅ DMG copied."
fi

# macOS App
APP_PATH="src-tauri/target/release/bundle/macos/*.app"
if compgen -G "$APP_PATH" > /dev/null; then
    cp -r $APP_PATH "$DIST_DIR/"
    echo "  ✅ .app copied."
fi

# Linux Deb
DEB_PATH="src-tauri/target/release/bundle/deb/*.deb"
if compgen -G "$DEB_PATH" > /dev/null; then
    cp $DEB_PATH "$DIST_DIR/"
    echo "  ✅ DEB copied."
fi

# Linux AppImage
APPIMAGE_PATH="src-tauri/target/release/bundle/appimage/*.AppImage"
if compgen -G "$APPIMAGE_PATH" > /dev/null; then
    cp $APPIMAGE_PATH "$DIST_DIR/"
    echo "  ✅ AppImage copied."
fi

echo "✨ Build complete! Artifacts are in the '$DIST_DIR' folder."
