#!/usr/bin/env bash
# Builds "DuckDNS Updater.app" (menu-bar only, no Dock icon).
#   ./scripts/bundle-macos.sh              -> for this Mac's CPU
#   ./scripts/bundle-macos.sh --universal  -> Apple Silicon + Intel
set -euo pipefail
cd "$(dirname "$0")/.."

APP="target/DuckDNS Updater.app"
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)

if [[ "${1:-}" == "--universal" ]]; then
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  cargo build --release --locked -p duckdns-tray --target aarch64-apple-darwin
  cargo build --release --locked -p duckdns-tray --target x86_64-apple-darwin
  BIN=target/duckdns-tray-universal
  lipo -create -output "$BIN" \
    target/aarch64-apple-darwin/release/duckdns-tray \
    target/x86_64-apple-darwin/release/duckdns-tray
else
  cargo build --release --locked -p duckdns-tray
  BIN=target/release/duckdns-tray
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/duckdns-tray"

# App icon: the same duck as the tray icon, drawn by examples/app_icon.rs.
ICONSET=target/AppIcon.iconset
rm -rf "$ICONSET"
cargo run --quiet --release --locked -p duckdns-tray --example app_icon -- "$ICONSET"
iconutil -c icns -o "$APP/Contents/Resources/AppIcon.icns" "$ICONSET"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>             <string>DuckDNS Updater</string>
    <key>CFBundleIdentifier</key>       <string>org.duckdns.updater</string>
    <key>CFBundleExecutable</key>       <string>duckdns-tray</string>
    <key>CFBundleIconFile</key>         <string>AppIcon</string>
    <key>CFBundlePackageType</key>      <string>APPL</string>
    <key>CFBundleVersion</key>          <string>${VERSION}</string>
    <key>CFBundleShortVersionString</key><string>${VERSION}</string>
    <key>LSMinimumSystemVersion</key>   <string>11.0</string>
    <key>LSUIElement</key>              <true/>
</dict>
</plist>
PLIST

# Ad-hoc signature (no Apple account needed); required to run on Apple Silicon.
codesign --force --deep --sign - "$APP"
echo "Built: $APP"
echo "Move it to /Applications, then open it once."
