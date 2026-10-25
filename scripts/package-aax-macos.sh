#!/bin/bash
# Stage an AAX-only preview with its required CLI server, then sign installers.
# Does not install anything, publish a release, or copy SDK sources.
set -euo pipefail
og_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$og_root"
: "${AAX_SDK_ROOT:?Provide the external AAX SDK root}"
: "${OPENGRIDDER_VST3_SDK:?Provide the external VST3 SDK root}"
og_python="${OG_RELEASE_PYTHON:-python3}"
og_version="1.2.1-preview.1"
og_stage="$og_root/target/release-stage/$og_version"
og_dist="$og_root/dist/$og_version"
og_aax="$og_root/target/aax-pace-signed/OpenGridder.aaxplugin"
og_server="$og_root/target/server-universal/OpenGridderServer"
og_application="${OPENGRIDDER_SIGN_IDENTITY:-Developer ID Application: Marcos Deida (ZQYU67JUBH)}"
og_installer="${OPENGRIDDER_INSTALL_IDENTITY:-Developer ID Installer: Marcos Deida (ZQYU67JUBH)}"
og_wraptool="${PACE_WRAPTOOL:-/Applications/PACEAntiPiracy/Eden/Fusion/Current/bin/wraptool}"
test -d "$og_aax" && test -x "$og_server"
test ! -e "$og_stage" || { echo "Stage already exists: $og_stage; choose a fresh build after archiving it" >&2; exit 1; }
mkdir -p "$og_stage/aax/Library/Application Support/Avid/Audio/Plug-Ins" \
    "$og_stage/server/Applications" "$og_stage/docs/Library/Application Support/OpenGridder" \
    "$og_stage/components" "$og_dist" "$og_stage/dmg/Documentation"
codesign --verify --deep --strict "$og_aax"
"$og_wraptool" verify --in "$og_aax" > "$og_dist/pace-verification.txt"
lipo "$og_aax/Contents/MacOS/OpenGridder" -verify_arch arm64 x86_64
lipo "$og_server" -verify_arch arm64 x86_64
ditto "$og_aax" "$og_stage/aax/Library/Application Support/Avid/Audio/Plug-Ins/OpenGridder.aaxplugin"
python3 scripts/stage-server.py --binary "$og_server" \
    --output "$og_stage/server/Applications/OpenGridder Server" --capabilities au-vst3
og_staged_server="$og_stage/server/Applications/OpenGridder Server"
codesign --force --timestamp --options runtime --entitlements native/server-entitlements.plist \
    --sign "$og_application" "$og_staged_server/OpenGridderServer"
codesign --verify --strict "$og_staged_server/OpenGridderServer"
"$og_staged_server/OpenGridderServer" --version
/usr/bin/arch -x86_64 "$og_staged_server/OpenGridderServer" --version
# Regenerate staged checksums after Developer ID signing changes the executable.
(cd "$og_staged_server" && shasum -a 256 OpenGridderServer 'Start OpenGridder Server.command' START-HERE.txt > SHA256SUMS)
"$og_python" tools/build_user_manual.py
python3 tools/collect_licenses.py --output "$og_stage/docs/Library/Application Support/OpenGridder/Licenses" \
    --aax-sdk "$AAX_SDK_ROOT" --vst3-sdk "$OPENGRIDDER_VST3_SDK"
cp output/pdf/OpenGridder-User-Manual.pdf RELEASE_NOTES.md "$og_stage/docs/Library/Application Support/OpenGridder/"
cp output/pdf/OpenGridder-User-Manual.pdf "$og_stage/dmg/Documentation/"
cp RELEASE_NOTES.md "$og_stage/dmg/"
ditto "$og_stage/docs/Library/Application Support/OpenGridder/Licenses" "$og_stage/dmg/Licenses"
cat > "$og_stage/dmg/INSTALL.txt" <<'TEXT'
OpenGridder 1.2.1 AAX preview - evaluation candidate
Quit Pro Tools, then open the installer. The AAX client and server are selectable.
For a second server computer choose Server and deselect the AAX client.
Start /Applications/OpenGridder Server/Start OpenGridder Server.command.
Read Documentation/OpenGridder-User-Manual.pdf before use.
Pro Tools host testing is pending. No PT10/OS X10.8.5 support is included.
No RTAS/TDM .dpm binaries or other client formats are included.
TEXT
pkgbuild --analyze --root "$og_stage/aax" "$og_stage/aax-bundles.plist"
python3 - "$og_stage/aax-bundles.plist" <<'PY'
import plistlib,sys
p=sys.argv[1]
with open(p,'rb') as f: bundles=plistlib.load(f)
for b in bundles:
    b['BundleIsRelocatable']=False
    b['BundleIsVersionChecked']=True
    b['BundleOverwriteAction']='upgrade'
with open(p,'wb') as f: plistlib.dump(bundles,f)
PY
pkgbuild --root "$og_stage/aax" --component-plist "$og_stage/aax-bundles.plist" \
    --identifier studio.opengrid.OpenGridder.aax.preview --version 1.2.1 \
    --install-location / --ownership recommended "$og_stage/components/AAX.pkg"
pkgbuild --root "$og_stage/server" --identifier studio.opengrid.OpenGridder.server.preview \
    --version 1.2.1 --install-location / --ownership recommended "$og_stage/components/Server.pkg"
pkgbuild --root "$og_stage/docs" --identifier studio.opengrid.OpenGridder.docs.preview \
    --version 1.2.1 --install-location / --ownership recommended "$og_stage/components/Docs.pkg"
cat > "$og_stage/distribution.xml" <<'XML'
<?xml version="1.0" encoding="utf-8"?>
<installer-gui-script minSpecVersion="2">
  <title>OpenGridder 1.2.1 AAX Preview</title>
  <options customize="always" require-scripts="false" hostArchitectures="x86_64,arm64"/>
  <domains enable_localSystem="true" enable_currentUserHome="false" enable_anywhere="false"/>
  <choices-outline><line choice="aax"/><line choice="server"/><line choice="docs"/></choices-outline>
  <choice id="aax" title="AAX Native client" description="64-bit mono/stereo AAX preview; Pro Tools host qualification pending. PT10 is not supported."><pkg-ref id="studio.opengrid.OpenGridder.aax.preview"/></choice>
  <choice id="server" title="OpenGridder Server" description="Required AU/VST3 effect server and Terminal launcher. Can be installed on a separate Mac."><pkg-ref id="studio.opengrid.OpenGridder.server.preview"/></choice>
  <choice id="docs" title="Manual and license notices" enabled="false" selected="true"><pkg-ref id="studio.opengrid.OpenGridder.docs.preview"/></choice>
  <pkg-ref id="studio.opengrid.OpenGridder.aax.preview" version="1.2.1">AAX.pkg</pkg-ref>
  <pkg-ref id="studio.opengrid.OpenGridder.server.preview" version="1.2.1">Server.pkg</pkg-ref>
  <pkg-ref id="studio.opengrid.OpenGridder.docs.preview" version="1.2.1">Docs.pkg</pkg-ref>
</installer-gui-script>
XML
productbuild --distribution "$og_stage/distribution.xml" --package-path "$og_stage/components" \
    --sign "$og_installer" "$og_dist/OpenGridder-$og_version-macos.pkg"
/usr/sbin/pkgutil --check-signature "$og_dist/OpenGridder-$og_version-macos.pkg"
echo "Signed preview installer: $og_dist"
echo "Next: bash scripts/notarize-aax-release-macos.sh"
