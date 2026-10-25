#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Multi-Format Release Packaging & Notarization for OpenGridder 0.1.0-alpha
set -euo pipefail

og_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$og_root"

og_version="0.1.0-alpha"
og_stage="$og_root/target/release-stage/$og_version"
og_dist="$og_root/dist/$og_version"

og_application="${OPENGRIDDER_SIGN_IDENTITY:-Developer ID Application: Marcos Deida (ZQYU67JUBH)}"
og_installer="${OPENGRIDDER_INSTALL_IDENTITY:-Developer ID Installer: Marcos Deida (ZQYU67JUBH)}"
og_notary_profile="${NOTARY_PROFILE:-AC_NOTARY}"
og_wraptool="${PACE_WRAPTOOL:-/Applications/PACEAntiPiracy/Eden/Fusion/Current/bin/wraptool}"

og_aax="$og_root/target/aax-pace-signed/OpenGridder.aaxplugin"
og_au="$og_root/target/au-universal/OpenGridder.component"
og_vst2="$og_root/target/vst2-universal/OpenGridder.vst"
og_server="$og_root/target/server-universal/OpenGridderServer"
og_win7_dir="$og_root/target/server-windows7"

echo "=== [1/7] Pre-Flight Verification ==="
test -d "$og_aax" || { echo "ERROR: Missing AAX plugin: $og_aax" >&2; exit 1; }
test -d "$og_au" || { echo "ERROR: Missing AU component: $og_au" >&2; exit 1; }
test -d "$og_vst2" || { echo "ERROR: Missing VST2 plugin: $og_vst2" >&2; exit 1; }
test -x "$og_server" || { echo "ERROR: Missing server binary: $og_server" >&2; exit 1; }
test -d "$og_win7_dir" || { echo "ERROR: Missing Windows 7 server dir: $og_win7_dir" >&2; exit 1; }

rm -rf "$og_stage"
mkdir -p "$og_stage/aax/Library/Application Support/Avid/Audio/Plug-Ins" \
         "$og_stage/au/Library/Audio/Plug-Ins/Components" \
         "$og_stage/vst2/Library/Audio/Plug-Ins/VST" \
         "$og_stage/server/Applications" \
         "$og_stage/docs/Library/Application Support/OpenGridder" \
         "$og_stage/components" \
         "$og_stage/dmg/Documentation" \
         "$og_dist"

echo "=== [2/7] Preparing and Codesigning Plugins with Hardened Runtime ==="
# Modern 64-bit universal (arm64 + x86_64) bundles for Notarized macOS installer
og_au_mod="$og_root/target/au-modern/OpenGridder.component"
rm -rf "$og_au_mod"
mkdir -p "$og_au_mod/Contents/MacOS"
ditto "$og_au" "$og_au_mod"
lipo -create \
    "$og_root/target/au-x86_64/OpenGridder.component/Contents/MacOS/OpenGridder" \
    "$og_root/target/au-arm64/OpenGridder.component/Contents/MacOS/OpenGridder" \
    -output "$og_au_mod/Contents/MacOS/OpenGridder"

og_vst2_mod="$og_root/target/vst2-modern/OpenGridder.vst"
rm -rf "$og_vst2_mod"
mkdir -p "$og_vst2_mod/Contents/MacOS"
ditto "$og_vst2" "$og_vst2_mod"
lipo -create \
    "$og_root/target/vst2-x86_64/OpenGridder.vst/Contents/MacOS/OpenGridder" \
    "$og_root/target/vst2-arm64/OpenGridder.vst/Contents/MacOS/OpenGridder" \
    -output "$og_vst2_mod/Contents/MacOS/OpenGridder"

codesign --force --timestamp --options runtime --sign "$og_application" "$og_au_mod"
codesign --verify --deep --strict "$og_au_mod"

codesign --force --timestamp --options runtime --sign "$og_application" "$og_vst2_mod"
codesign --verify --deep --strict "$og_vst2_mod"

"$og_wraptool" verify --in "$og_aax" > "$og_dist/pace-verification.txt"
codesign --verify --deep --strict "$og_aax"

ditto "$og_aax" "$og_stage/aax/Library/Application Support/Avid/Audio/Plug-Ins/OpenGridder.aaxplugin"
ditto "$og_au_mod" "$og_stage/au/Library/Audio/Plug-Ins/Components/OpenGridder.component"
ditto "$og_vst2_mod" "$og_stage/vst2/Library/Audio/Plug-Ins/VST/OpenGridder.vst"

echo "=== [3/7] Staging and Signing Server ==="
python3 scripts/stage-server.py --binary "$og_server" \
    --output "$og_stage/server/Applications/OpenGridder Server" --capabilities au-vst3
og_staged_server="$og_stage/server/Applications/OpenGridder Server"
codesign --force --timestamp --options runtime --entitlements native/server-entitlements.plist \
    --sign "$og_application" "$og_staged_server/OpenGridderServer"
codesign --verify --strict "$og_staged_server/OpenGridderServer"

(cd "$og_staged_server" && shasum -a 256 OpenGridderServer 'Start OpenGridder Server.command' START-HERE.txt > SHA256SUMS)

echo "=== [4/7] Documentation and Licenses ==="
python3 tools/build_user_manual.py
cp output/pdf/OpenGridder-User-Manual.pdf "$og_stage/docs/Library/Application Support/OpenGridder/"
cp RELEASE_NOTES.md README.md "$og_stage/docs/Library/Application Support/OpenGridder/" 2>/dev/null || true
cp output/pdf/OpenGridder-User-Manual.pdf "$og_stage/dmg/Documentation/"
cp RELEASE_NOTES.md README.md "$og_stage/dmg/Documentation/" 2>/dev/null || true

if [ -d "$og_root/target/release-licenses" ]; then
    ditto "$og_root/target/release-licenses" "$og_stage/docs/Library/Application Support/OpenGridder/Licenses"
    ditto "$og_root/target/release-licenses" "$og_stage/dmg/Licenses"
fi

cat > "$og_stage/dmg/INSTALL.txt" << 'EOF'
OpenGridder 0.1.0-alpha - Multi-Format Release
==============================================

Welcome to OpenGridder, the zero-JUCE audio streaming plugin network.

Included Plugins:
- AAX Native: Pro Tools 11+ and Pro Tools Ultimate (Apple Silicon & Intel)
- AudioUnit v2: Logic Pro 8 (Component Manager), Logic Pro 9, Logic Pro X, GarageBand
- VST 2.4: Cubase 4/5/6/7, Nuendo, WaveLab, Reaper

Server Components:
- macOS Universal Server: Installed to /Applications/OpenGridder Server/
- Windows 7 SP1+ Server: Packaged separately as OpenGridder-0.1.0-alpha-windows7-x86_64.zip

Quick Start:
1. Run the OpenGridder-0.1.0-alpha-macos.pkg installer and choose your DAW formats.
2. Launch /Applications/OpenGridder Server/Start OpenGridder Server.command.
3. Open your DAW and insert OpenGridder on an audio track.
4. Select your latency profile (Profile 0: LAN, Profile 1: 10GbE, Profile 2: VSTi, Profile 3: Direct).
EOF

echo "=== [5/7] Building Component Packages ==="
pkgbuild --root "$og_stage/aax" --identifier studio.opengrid.OpenGridder.aax --version "$og_version" \
    --install-location / --ownership recommended "$og_stage/components/AAX.pkg"

pkgbuild --root "$og_stage/au" --identifier studio.opengrid.OpenGridder.au --version "$og_version" \
    --install-location / --ownership recommended "$og_stage/components/AU.pkg"

pkgbuild --root "$og_stage/vst2" --identifier studio.opengrid.OpenGridder.vst2 --version "$og_version" \
    --install-location / --ownership recommended "$og_stage/components/VST2.pkg"

pkgbuild --root "$og_stage/server" --identifier studio.opengrid.OpenGridder.server --version "$og_version" \
    --install-location / --ownership recommended "$og_stage/components/Server.pkg"

pkgbuild --root "$og_stage/docs" --identifier studio.opengrid.OpenGridder.docs --version "$og_version" \
    --install-location / --ownership recommended "$og_stage/components/Docs.pkg"

cat > "$og_stage/distribution.xml" << EOF
<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="2">
  <title>OpenGridder $og_version</title>
  <options customize="always" require-scripts="false" hostArchitectures="x86_64,arm64"/>
  <domains enable_localSystem="true" enable_currentUserHome="false" enable_anywhere="false"/>
  <choices-outline>
    <line choice="aax"/>
    <line choice="au"/>
    <line choice="vst2"/>
    <line choice="server"/>
    <line choice="docs"/>
  </choices-outline>
  <choice id="aax" title="AAX Native Plugin" description="Avid AAX plugin for Pro Tools 11+ and Pro Tools Ultimate.">
    <pkg-ref id="studio.opengrid.OpenGridder.aax"/>
  </choice>
  <choice id="au" title="AudioUnit v2 Plugin" description="Universal AU plugin supporting Logic Pro 8 (Component Manager), Logic Pro 9, and Logic Pro X.">
    <pkg-ref id="studio.opengrid.OpenGridder.au"/>
  </choice>
  <choice id="vst2" title="VST 2.4 Plugin" description="Universal VST2 plugin for Cubase 4/5/6/7, Nuendo, and vintage Mac DAWs.">
    <pkg-ref id="studio.opengrid.OpenGridder.vst2"/>
  </choice>
  <choice id="server" title="OpenGridder Server" description="Universal macOS plugin server and launcher.">
    <pkg-ref id="studio.opengrid.OpenGridder.server"/>
  </choice>
  <choice id="docs" title="Manual and Licenses" enabled="false" selected="true">
    <pkg-ref id="studio.opengrid.OpenGridder.docs"/>
  </choice>
  <pkg-ref id="studio.opengrid.OpenGridder.aax" version="$og_version">AAX.pkg</pkg-ref>
  <pkg-ref id="studio.opengrid.OpenGridder.au" version="$og_version">AU.pkg</pkg-ref>
  <pkg-ref id="studio.opengrid.OpenGridder.vst2" version="$og_version">VST2.pkg</pkg-ref>
  <pkg-ref id="studio.opengrid.OpenGridder.server" version="$og_version">Server.pkg</pkg-ref>
  <pkg-ref id="studio.opengrid.OpenGridder.docs" version="$og_version">Docs.pkg</pkg-ref>
</installer-gui-script>
EOF

FINAL_PKG="$og_dist/OpenGridder-$og_version-macos.pkg"
productbuild --distribution "$og_stage/distribution.xml" --package-path "$og_stage/components" \
    --sign "$og_installer" "$FINAL_PKG"

/usr/sbin/pkgutil --check-signature "$FINAL_PKG"

echo "=== [6/7] Notarizing Package and Building Notarized DMG ==="
xcrun notarytool submit "$FINAL_PKG" --keychain-profile "$og_notary_profile" --wait
xcrun stapler staple "$FINAL_PKG"
spctl --assess --type install -v "$FINAL_PKG"

ditto "$FINAL_PKG" "$og_stage/dmg/OpenGridder-$og_version-macos.pkg"

FINAL_DMG="$og_dist/OpenGridder-$og_version-macos.dmg"
/Users/marcosdeida/.gemini/config/skills/Make-Plugin-Release/scripts/build_notarized_dmg.sh \
    "OpenGridder $og_version" \
    "$og_stage/dmg" \
    "$FINAL_DMG" \
    "$og_application" \
    "$og_notary_profile"

echo "=== [7/7] Packaging Windows 7 Server & Vintage Mac ZIPs ==="
WIN7_ZIP="$og_dist/OpenGridder-$og_version-windows7-x86_64.zip"
(cd "$og_win7_dir" && zip -r -9 "$WIN7_ZIP" .)

# Legacy Vintage Mac Bundle (i386 + x86_64 + arm64 for Logic 8/9 & vintage Cubase on OS X 10.6-10.14)
LEGACY_DIR="$og_stage/legacy-mac"
mkdir -p "$LEGACY_DIR/Components" "$LEGACY_DIR/VST" "$LEGACY_DIR/Documentation"
ditto "$og_au" "$LEGACY_DIR/Components/OpenGridder.component"
ditto "$og_vst2" "$LEGACY_DIR/VST/OpenGridder.vst"
ditto "$og_root/target/server-universal/OpenGridderServer" "$LEGACY_DIR/OpenGridderServer"
cp "$og_root/output/pdf/OpenGridder-User-Manual.pdf" "$LEGACY_DIR/Documentation/"

cat > "$LEGACY_DIR/README-Vintage-Mac.txt" << 'EOF'
OpenGridder Vintage Mac OS X Package (i386 + x86_64)
===================================================

Supported Vintage Systems:
- OS X 10.6 (Snow Leopard), 10.7 (Lion), 10.8 (Mountain Lion),
  10.9 (Mavericks), 10.10 (Yosemite), 10.11 (El Capitan), 10.14 (Mojave).

Included Plugins:
- Components/OpenGridder.component:
  Universal AudioUnit (i386 + x86_64) containing the Carbon Component Manager
  'thng' resource for Logic Pro 8 (32-bit), Logic Pro 9 (32-bit/64-bit), and modern DAWs.
  Installation: Copy to /Library/Audio/Plug-Ins/Components/

- VST/OpenGridder.vst:
  Universal VST 2.4 plugin (i386 + x86_64) for Cubase 4/5/6/7, Nuendo 3/4/5, and WaveLab.
  Installation: Copy to /Library/Audio/Plug-Ins/VST/

- OpenGridderServer:
  Universal CLI server for hosting and streaming.
EOF

LEGACY_ZIP="$og_dist/OpenGridder-$og_version-legacy-vintage-mac.zip"
(cd "$LEGACY_DIR" && zip -r -9 "$LEGACY_ZIP" .)

echo "=== Computing SHA256 Checksums ==="
(cd "$og_dist" && shasum -a 256 *.pkg *.dmg *.zip > SHA256SUMS)

echo "======================================================="
echo "SUCCESS: OpenGridder $og_version Release Complete!"
echo "Release Artifacts Directory: $og_dist"
ls -lh "$og_dist"
cat "$og_dist/SHA256SUMS"
echo "======================================================="
