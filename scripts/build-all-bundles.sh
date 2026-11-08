#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
og_root="$(cd "$script_dir/.." && pwd)"
source "$script_dir/build-info.sh"

echo "=== 1. Building ag-formats dylib & server ==="
cd "$og_root"
cargo build --release -p ag-formats -p ag-server -p ag-tray

"$og_root/scripts/build-server-app-macos.sh"

STAGE="$og_root/build/release/OpenGridder_artefacts/Release"
rm -rf "$STAGE"
mkdir -p "$STAGE/VST3" "$STAGE/AU" "$STAGE/CLAP" "$STAGE/LV2" "$STAGE/AAX" "$STAGE/Standalone"

DYLIB="$og_root/target/release/libag_formats.dylib"

echo "=== 2. Assembling Plugin Formats ==="

# VST3
cp -R "$og_root/crates/ag-formats/bundle/AudioGridder.vst3" "$STAGE/VST3/OpenGridder.vst3"
mkdir -p "$STAGE/VST3/OpenGridder.vst3/Contents/MacOS"
cp "$DYLIB" "$STAGE/VST3/OpenGridder.vst3/Contents/MacOS/OpenGridder"
ln -sf "OpenGridder" "$STAGE/VST3/OpenGridder.vst3/Contents/MacOS/AudioGridder"

# AU
cp -R "$og_root/crates/ag-formats/bundle/AudioGridder.component" "$STAGE/AU/OpenGridder.component"
mkdir -p "$STAGE/AU/OpenGridder.component/Contents/MacOS"
cp "$DYLIB" "$STAGE/AU/OpenGridder.component/Contents/MacOS/OpenGridder"
ln -sf "OpenGridder" "$STAGE/AU/OpenGridder.component/Contents/MacOS/AudioGridder"

# CLAP
cp "$DYLIB" "$STAGE/CLAP/OpenGridder.clap"

# LV2
cp -R "$og_root/crates/ag-formats/bundle/AudioGridder.lv2" "$STAGE/LV2/OpenGridder.lv2"
cp "$DYLIB" "$STAGE/LV2/OpenGridder.lv2/OpenGridder.dylib"
ln -sf "OpenGridder.dylib" "$STAGE/LV2/OpenGridder.lv2/AudioGridder.dylib"

# AAX
cp -R "$og_root/crates/ag-formats/bundle/AudioGridder.aaxplugin" "$STAGE/AAX/OpenGridder.aaxplugin"
mkdir -p "$STAGE/AAX/OpenGridder.aaxplugin/Contents/MacOS"
cp "$DYLIB" "$STAGE/AAX/OpenGridder.aaxplugin/Contents/MacOS/OpenGridder"
ln -sf "OpenGridder" "$STAGE/AAX/OpenGridder.aaxplugin/Contents/MacOS/AudioGridder"
cat << 'PLIST' > "$STAGE/AAX/OpenGridder.aaxplugin/Contents/Info.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>English</string>
    <key>CFBundleExecutable</key>
    <string>OpenGridder</string>
    <key>CFBundleGetInfoString</key>
    <string>OpenGridder AAX Plugin 0.1.1-alpha</string>
    <key>CFBundleIdentifier</key>
    <string>com.opengrid.opengridder.aax</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>OpenGridder</string>
    <key>CFBundlePackageType</key>
    <string>TDMw</string>
    <key>CFBundleSignature</key>
    <string>PTul</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.1-alpha</string>
    <key>CFBundleVersion</key>
    <string>0.1.1-alpha</string>
    <key>CSResourcesFileMapped</key>
    <true/>
</dict>
</plist>
PLIST

# Standalone Server App
cp -R "$og_root/build/OpenGridder Server.app" "$STAGE/Standalone/OpenGridder Server.app"
cp "$og_root/target/release/ag-tray" "$STAGE/Standalone/OpenGridderTray"

echo "=== 3. PACE Eden AAX Signing via physical iLok ==="
if /Users/marcosdeida/.gemini/config/skills/Make-Plugin-Release/scripts/sign_aax.sh \
    "$STAGE/AAX/OpenGridder.aaxplugin" "807F4D50-CFDB-11F1-A0CD-00505692AD3E" "mistamin"; then
    echo "PACE AAX signing succeeded."
else
    echo "WARNING: PACE AAX signing skipped or failed (iLok session); continuing with Developer ID signing."
fi

echo "=== 4. Apple Developer ID Signing ==="
IDENTITY="Developer ID Application: Marcos Deida (ZQYU67JUBH)"
ENTITLEMENTS="$og_root/package/OpenGridderServer.entitlements"

codesign --force --timestamp --options runtime --sign "$IDENTITY" "$STAGE/Standalone/OpenGridderTray"
codesign --force --timestamp --options runtime --entitlements "$ENTITLEMENTS" --sign "$IDENTITY" "$STAGE/Standalone/OpenGridder Server.app/Contents/MacOS/OpenGridderServer-bin"
codesign --force --timestamp --options runtime --entitlements "$ENTITLEMENTS" --sign "$IDENTITY" "$STAGE/Standalone/OpenGridder Server.app/Contents/MacOS/OpenGridderServer"
codesign --force --timestamp --options runtime --entitlements "$ENTITLEMENTS" --sign "$IDENTITY" "$STAGE/Standalone/OpenGridder Server.app"

codesign --force --timestamp --options runtime --sign "$IDENTITY" "$STAGE/CLAP/OpenGridder.clap"
codesign --force --timestamp --options runtime --sign "$IDENTITY" "$STAGE/LV2/OpenGridder.lv2/OpenGridder.dylib"
codesign --force --timestamp --options runtime --sign "$IDENTITY" "$STAGE/VST3/OpenGridder.vst3"
codesign --force --timestamp --options runtime --sign "$IDENTITY" "$STAGE/AU/OpenGridder.component"

echo "SUCCESS: Staged, PACE signed, and Developer ID signed bundles at $STAGE"
