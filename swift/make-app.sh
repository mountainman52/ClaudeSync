#!/usr/bin/env bash
# Builds ClaudeSync.app — a menu-bar-only (LSUIElement) bundle — from the
# SwiftPM package. Run on macOS:
#
#   ./make-app.sh
#   mv ClaudeSync.app /Applications/
set -euo pipefail
cd "$(dirname "$0")"

swift build -c release

APP="ClaudeSync.app"
BIN=".build/release/ClaudeSyncBar"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp "$BIN" "$APP/Contents/MacOS/ClaudeSyncBar"

cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>ClaudeSyncBar</string>
    <key>CFBundleIdentifier</key>
    <string>com.claudesync.menubar</string>
    <key>CFBundleName</key>
    <string>ClaudeSync</string>
    <key>CFBundleDisplayName</key>
    <string>ClaudeSync</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

# Ad-hoc signature keeps Keychain access stable between launches
codesign --force --sign - "$APP" 2>/dev/null \
    || echo "note: ad-hoc codesign failed; the app will still run"

echo "Built $APP"
echo "Install with: mv $APP /Applications/  (then open it from Applications)"
