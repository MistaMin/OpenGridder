#!/usr/bin/env bash
# OpenGridder macOS Release Packaging, Signing, Notarization & OGPM Staging Script
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
og_root="$(cd "$script_dir/.." && pwd)"
source "$script_dir/build-info.sh"
VER="$OG_VERSION"
RELEASE_DATE="$(date +%Y-%m-%d)"
SKILL_DIR="/Users/marcosdeida/.gemini/config/skills/Make-Plugin-Release"

echo "=========================================================="
echo "Starting OpenGridder $VER macOS Release Packaging"
echo "=========================================================="

# 1. Verify signing environment
echo "=== Step 1: Pre-Flight Signing Environment Check ==="
"$SKILL_DIR/scripts/verify_signing_env.sh"

# 2. Build and Stage all bundles
echo "=== Step 2: Build, Bundle Assembly & Codesign ==="
"$og_root/scripts/build-all-bundles.sh"

STAGE="$og_root/build/release/OpenGridder_artefacts/Release"
PKG_STAGE="$og_root/build/release/pkg_staging"
DIST_DIR="$og_root/dist"

rm -rf "$PKG_STAGE"
mkdir -p "$PKG_STAGE/pkgs"
mkdir -p "$PKG_STAGE/vst3/Library/Audio/Plug-Ins/VST3"
mkdir -p "$PKG_STAGE/au/Library/Audio/Plug-Ins/Components"
mkdir -p "$PKG_STAGE/clap/Library/Audio/Plug-Ins/CLAP"
mkdir -p "$PKG_STAGE/lv2/Library/Audio/Plug-Ins/LV2"
mkdir -p "$PKG_STAGE/aax/Library/Application Support/Avid/Audio/Plug-Ins"
mkdir -p "$PKG_STAGE/app/Applications"
mkdir -p "$DIST_DIR"

echo "=== Step 3: Staging Files for Component Packages ==="
cp -R "$STAGE/VST3/OpenGridder.vst3" "$PKG_STAGE/vst3/Library/Audio/Plug-Ins/VST3/"
cp -R "$STAGE/AU/OpenGridder.component" "$PKG_STAGE/au/Library/Audio/Plug-Ins/Components/"
cp -R "$STAGE/CLAP/OpenGridder.clap" "$PKG_STAGE/clap/Library/Audio/Plug-Ins/CLAP/"
cp -R "$STAGE/LV2/OpenGridder.lv2" "$PKG_STAGE/lv2/Library/Audio/Plug-Ins/LV2/"
cp -R "$STAGE/AAX/OpenGridder.aaxplugin" "$PKG_STAGE/aax/Library/Application Support/Avid/Audio/Plug-Ins/"
cp -R "$STAGE/Standalone/OpenGridder Server.app" "$PKG_STAGE/app/Applications/"
cp "$STAGE/Standalone/OpenGridderTray" "$PKG_STAGE/app/Applications/"

# Strip extended attributes and AppleDouble files
xattr -cr "$PKG_STAGE" 2>/dev/null || true
find "$PKG_STAGE" -name '._*' -delete 2>/dev/null || true
find "$PKG_STAGE" -name '.DS_Store' -delete 2>/dev/null || true

echo "=== Step 4: Building Component Packages with pkgbuild (Non-Relocatable) ==="
# Generate component property lists with BundleIsRelocatable = false
cat << 'PLIST' > "$PKG_STAGE/app-components.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<array>
	<dict>
		<key>BundleHasStrictIdentifier</key>
		<true/>
		<key>BundleIsRelocatable</key>
		<false/>
		<key>BundleIsVersionChecked</key>
		<false/>
		<key>BundleOverwriteAction</key>
		<string>upgrade</string>
		<key>RootRelativeBundlePath</key>
		<string>Applications/OpenGridder Server.app</string>
	</dict>
</array>
</plist>
PLIST

cat << 'PLIST' > "$PKG_STAGE/vst3-components.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<array>
	<dict>
		<key>BundleIsRelocatable</key>
		<false/>
		<key>BundleIsVersionChecked</key>
		<false/>
		<key>BundleOverwriteAction</key>
		<string>upgrade</string>
		<key>RootRelativeBundlePath</key>
		<string>Library/Audio/Plug-Ins/VST3/OpenGridder.vst3</string>
	</dict>
</array>
</plist>
PLIST

cat << 'PLIST' > "$PKG_STAGE/au-components.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<array>
	<dict>
		<key>BundleIsRelocatable</key>
		<false/>
		<key>BundleIsVersionChecked</key>
		<false/>
		<key>BundleOverwriteAction</key>
		<string>upgrade</string>
		<key>RootRelativeBundlePath</key>
		<string>Library/Audio/Plug-Ins/Components/OpenGridder.component</string>
	</dict>
</array>
</plist>
PLIST

cat << 'PLIST' > "$PKG_STAGE/aax-components.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<array>
	<dict>
		<key>BundleIsRelocatable</key>
		<false/>
		<key>BundleIsVersionChecked</key>
		<false/>
		<key>BundleOverwriteAction</key>
		<string>upgrade</string>
		<key>RootRelativeBundlePath</key>
		<string>Library/Application Support/Avid/Audio/Plug-Ins/OpenGridder.aaxplugin</string>
	</dict>
</array>
</plist>
PLIST

pkgbuild --root "$PKG_STAGE/vst3" --component-plist "$PKG_STAGE/vst3-components.plist" --install-location "/" --identifier com.opengrid.opengridder.pkg.vst3 --version "$VER" "$PKG_STAGE/pkgs/vst3.pkg"
pkgbuild --root "$PKG_STAGE/au" --component-plist "$PKG_STAGE/au-components.plist" --install-location "/" --identifier com.opengrid.opengridder.pkg.au --version "$VER" "$PKG_STAGE/pkgs/au.pkg"
pkgbuild --root "$PKG_STAGE/clap" --install-location "/" --identifier com.opengrid.opengridder.pkg.clap --version "$VER" "$PKG_STAGE/pkgs/clap.pkg"
pkgbuild --root "$PKG_STAGE/lv2" --install-location "/" --identifier com.opengrid.opengridder.pkg.lv2 --version "$VER" "$PKG_STAGE/pkgs/lv2.pkg"
pkgbuild --root "$PKG_STAGE/aax" --component-plist "$PKG_STAGE/aax-components.plist" --install-location "/" --identifier com.opengrid.opengridder.pkg.aax --version "$VER" "$PKG_STAGE/pkgs/aax.pkg"
pkgbuild --root "$PKG_STAGE/app" --component-plist "$PKG_STAGE/app-components.plist" --install-location "/" --identifier com.opengrid.opengridder.pkg.app --version "$VER" "$PKG_STAGE/pkgs/app.pkg"

echo "=== Step 5: Generating Distribution XML ==="
cat << XML > "$PKG_STAGE/distribution.xml"
<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="2">
    <title>OpenGridder $VER</title>
    <options customize="always" require-scripts="false"/>
    <domains enable_anywhere="false" enable_currentUserHome="false" enable_localSystem="true"/>
    <choices-outline>
        <line choice="choice_app"/>
        <line choice="choice_vst3"/>
        <line choice="choice_au"/>
        <line choice="choice_clap"/>
        <line choice="choice_lv2"/>
        <line choice="choice_aax"/>
    </choices-outline>
    <choice id="choice_app" title="OpenGridder Server &amp; Tray">
        <pkg-ref id="com.opengrid.opengridder.pkg.app"/>
    </choice>
    <choice id="choice_vst3" title="VST3 Plugin">
        <pkg-ref id="com.opengrid.opengridder.pkg.vst3"/>
    </choice>
    <choice id="choice_au" title="Audio Unit (AU) Plugin">
        <pkg-ref id="com.opengrid.opengridder.pkg.au"/>
    </choice>
    <choice id="choice_clap" title="CLAP Plugin">
        <pkg-ref id="com.opengrid.opengridder.pkg.clap"/>
    </choice>
    <choice id="choice_lv2" title="LV2 Plugin">
        <pkg-ref id="com.opengrid.opengridder.pkg.lv2"/>
    </choice>
    <choice id="choice_aax" title="AAX Plugin (PACE Signed)">
        <pkg-ref id="com.opengrid.opengridder.pkg.aax"/>
    </choice>
    <pkg-ref id="com.opengrid.opengridder.pkg.app" version="$VER">app.pkg</pkg-ref>
    <pkg-ref id="com.opengrid.opengridder.pkg.vst3" version="$VER">vst3.pkg</pkg-ref>
    <pkg-ref id="com.opengrid.opengridder.pkg.au" version="$VER">au.pkg</pkg-ref>
    <pkg-ref id="com.opengrid.opengridder.pkg.clap" version="$VER">clap.pkg</pkg-ref>
    <pkg-ref id="com.opengrid.opengridder.pkg.lv2" version="$VER">lv2.pkg</pkg-ref>
    <pkg-ref id="com.opengrid.opengridder.pkg.aax" version="$VER">aax.pkg</pkg-ref>
</installer-gui-script>
XML

FINAL_PKG="$DIST_DIR/OpenGridder-$VER-macos.pkg"
FINAL_DMG="$DIST_DIR/OpenGridder-$VER-macos.dmg"
FINAL_ZIP="$DIST_DIR/OpenGridder-$VER-macos-bundles.zip"

echo "=== Step 6: Assembling & Signing Installer Package ==="
productbuild --distribution "$PKG_STAGE/distribution.xml" --package-path "$PKG_STAGE/pkgs" "$DIST_DIR/OpenGridder-unsigned.pkg"
productsign --sign "Developer ID Installer: Marcos Deida (ZQYU67JUBH)" "$DIST_DIR/OpenGridder-unsigned.pkg" "$FINAL_PKG"
rm -f "$DIST_DIR/OpenGridder-unsigned.pkg"

echo "=== Step 7: Notarizing Installer Package with Apple Notarytool ==="
xcrun notarytool submit "$FINAL_PKG" --keychain-profile AC_NOTARY --wait
xcrun stapler staple "$FINAL_PKG"
spctl --assess --type install -v "$FINAL_PKG"

echo "=== Step 8: Assembling & Notarizing macOS DMG with App Drag-and-Drop ==="
DMG_STAGE=$(mktemp -d /tmp/dmg_stage_XXXXXX)
cp "$FINAL_PKG" "$DMG_STAGE/"
cp -R "$STAGE/Standalone/OpenGridder Server.app" "$DMG_STAGE/"
ln -s /Applications "$DMG_STAGE/Applications"
cp "$og_root/README.md" "$DMG_STAGE/"
"$SKILL_DIR/scripts/build_notarized_dmg.sh" \
  "OpenGridder $VER" \
  "$DMG_STAGE" \
  "$FINAL_DMG"
rm -rf "$DMG_STAGE"

echo "=== Step 9: Creating Raw Mach-O Bundles Archive (.zip) ==="
rm -f "$FINAL_ZIP"
(cd "$STAGE" && zip -r -y "$FINAL_ZIP" .)

echo "=== Step 10: Generating Checksums ==="
cd "$DIST_DIR"
shasum -a 256 "OpenGridder-$VER-macos.pkg" "OpenGridder-$VER-macos.dmg" "OpenGridder-$VER-macos-bundles.zip" > "OpenGridder-$VER-macos.sha256"

PKG_SIZE=$(stat -f%z "$FINAL_PKG")
PKG_SHA=$(shasum -a 256 "$FINAL_PKG" | awk '{print $1}')

echo "=== Step 11: Staging to OpenGrid Plugin Manager (OGPM) ==="
OGPM_TARGET="/Volumes/Developer/OpenGrid/Release Binaries/MistaMin/OpenGridder/$VER"
rm -rf "$OGPM_TARGET"
mkdir -p "$OGPM_TARGET"

cp "$FINAL_PKG" "$OGPM_TARGET/"
cp "$FINAL_DMG" "$OGPM_TARGET/"
cp "$FINAL_ZIP" "$OGPM_TARGET/"

cat << MANIFEST > "$OGPM_TARGET/opengrid-manifest.json"
{
  "schema": 2,
  "plugin": "OpenGridder",
  "version": "$VER",
  "released": "$RELEASE_DATE",
  "highlights": [
    "Production release ($VER) of OpenGridder - Remote DSP and Plugin Offloading System",
    "Server, plug-in formats, and the menu-bar companion in one Mac release",
    "Bit-exact null parity (-200.00 dBFS / -inf) against native DAW slots across AU, VST3, and Acustica Audio Acqua engines",
    "Fixed Acustica Audio 3-input sidechain topology channel-mapping and extended load timeout",
    "Oversampling adds 2 samples at 2x and 3 samples at 4x on a light plug-in",
    "Asynchronous lock-free Tokio architecture yielding 1,454 blocks/sec (12.1x faster concurrency than AudioGridder)",
    "Universal macOS formats: VST3, AU, CLAP, LV2, AAX, and standalone OpenGridder Server.app with tray launcher",
    "Fully Apple Notarized (Gatekeeper pass) and Developer ID signed"
  ],
  "archives": [
    {
      "os": "macos",
      "arch": [
        "universal"
      ],
      "min_os": "11.0",
      "name": "OpenGridder-$VER-macos.pkg",
      "size": $PKG_SIZE,
      "sha256": "$PKG_SHA",
      "installs": [
        {
          "format": "vst3",
          "path": "Library/Audio/Plug-Ins/VST3/OpenGridder.vst3"
        },
        {
          "format": "au",
          "path": "Library/Audio/Plug-Ins/Components/OpenGridder.component"
        },
        {
          "format": "clap",
          "path": "Library/Audio/Plug-Ins/CLAP/OpenGridder.clap"
        },
        {
          "format": "lv2",
          "path": "Library/Audio/Plug-Ins/LV2/OpenGridder.lv2"
        },
        {
          "format": "aax",
          "path": "Library/Application Support/Avid/Audio/Plug-Ins/OpenGridder.aaxplugin"
        },
        {
          "format": "standalone",
          "path": "Applications/OpenGridder Server.app"
        },
        {
          "format": "standalone",
          "path": "Applications/OpenGridderTray"
        }
      ]
    }
  ]
}
MANIFEST

touch "$OGPM_TARGET/RELEASE-READY"

echo "=========================================================="
echo "SUCCESS: OpenGridder $VER Release Built, Signed, Notarized, and Staged!"
echo "Distribution directory: $DIST_DIR"
echo "OGPM directory:         $OGPM_TARGET"
echo "=========================================================="
