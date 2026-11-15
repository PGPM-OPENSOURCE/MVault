#!/bin/sh
# Builds "Memoro Vault.app" and a DMG from the Rust sources (macOS).
#   scripts/package-macos.sh            -> dist/Memoro Vault.app, dist/Memoro-Vault-<version>.dmg
# The app is signed ad hoc (no Developer ID): on first launch from a download, macOS asks to
# confirm opening it (right-click > Open).
set -eu
cd "$(dirname "$0")/.."

cargo build -p mvault --release

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
OUT=dist
APP="$OUT/Memoro Vault.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/mvault "$APP/Contents/MacOS/mvault"
cp app/assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>Memoro Vault</string>
  <key>CFBundleExecutable</key><string>mvault</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundleIdentifier</key><string>com.memorovault.app</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>Memoro Vault</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>LSMinimumSystemVersion</key><string>10.15</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>GPL-3.0</string>
  <key>NSCameraUsageDescription</key><string>The camera is used only to scan the QR codes of a paper backup.</string>
</dict>
</plist>
PLIST
codesign --force --deep --sign - "$APP"
codesign --verify --strict "$APP"

STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f "$OUT/Memoro-Vault-$VERSION.dmg"
hdiutil create -quiet -volname "Memoro Vault" -srcfolder "$STAGE" -ov -format UDZO "$OUT/Memoro-Vault-$VERSION.dmg"
rm -rf "$STAGE"
echo "built: $APP"
echo "built: $OUT/Memoro-Vault-$VERSION.dmg"
