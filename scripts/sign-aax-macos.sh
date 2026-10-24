#!/usr/bin/env bash
# OpenGridder Signing Only configuration. No credentials are stored here.
set -euo pipefail
og_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
og_input="$og_root/target/aax-universal/OpenGridder.aaxplugin"
og_destination="$og_root/target/aax-pace-signed"
og_wraptool="${PACE_WRAPTOOL:-/Applications/PACEAntiPiracy/Eden/Fusion/Current/bin/wraptool}"
og_identity="${OPENGRIDDER_SIGN_IDENTITY:-Developer ID Application: Marcos Deida (ZQYU67JUBH)}"
og_account="${OPENGRIDDER_PACE_ACCOUNT:-mistamin}"
og_guid="${OPENGRIDDER_PACE_GUID:-807F4D50-CFDB-11F1-A0CD-00505692AD3E}"
test -d "$og_input" && test -x "$og_wraptool"
og_stage="$(mktemp -d "$og_root/target/pace-stage.XXXXXX")"
trap 'rm -rf "$og_stage"' EXIT
mkdir -p "$og_stage/input" "$og_stage/output"
ditto "$og_input" "$og_stage/input/OpenGridder.aaxplugin"
codesign --force --deep --timestamp --options runtime --sign "$og_identity" "$og_stage/input/OpenGridder.aaxplugin"
"$og_wraptool" sign --verbose --account "$og_account" --wcguid "$og_guid" \
    --signid "$og_identity" --in "$og_stage/input/OpenGridder.aaxplugin" \
    --out "$og_stage/output/OpenGridder.aaxplugin" --autoinstall on
# Seal the final PACE metadata with the Apple hardened-runtime signature.
# PACE verification follows this pass to establish that both signatures coexist.
codesign --force --deep --timestamp --options runtime --sign "$og_identity" "$og_stage/output/OpenGridder.aaxplugin"
codesign --verify --deep --strict --verbose=2 "$og_stage/output/OpenGridder.aaxplugin"
"$og_wraptool" verify --verbose --in "$og_stage/output/OpenGridder.aaxplugin"
lipo "$og_stage/output/OpenGridder.aaxplugin/Contents/MacOS/OpenGridder" -verify_arch arm64 x86_64
mkdir -p "$og_destination"
# This is a generated signing output, kept separate from the development bundle.
rm -rf "$og_destination/OpenGridder.aaxplugin"
ditto "$og_stage/output/OpenGridder.aaxplugin" "$og_destination/OpenGridder.aaxplugin"
shasum -a 256 "$og_destination/OpenGridder.aaxplugin/Contents/MacOS/OpenGridder" \
    "$og_destination/OpenGridder.aaxplugin/Contents/Info.plist" > "$og_destination/SHA256SUMS"
echo "Verified PACE/Apple signed AAX: $og_destination/OpenGridder.aaxplugin"
