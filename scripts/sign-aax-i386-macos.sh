#!/bin/bash
# Separate experimental PT10 artifact. Keep PACE version-1 signature compatibility.
set -euo pipefail
og_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
og_identity="${OPENGRIDDER_SIGN_IDENTITY:-Developer ID Application: Marcos Deida (ZQYU67JUBH)}"
og_wraptool="${PACE_WRAPTOOL:-/Applications/PACEAntiPiracy/Eden/Fusion/Current/bin/wraptool}"
og_output="$og_root/target/aax-i386-pace-signed"
og_stage="$(mktemp -d "$og_root/target/pace-i386-stage.XXXXXX")"
trap 'rm -rf "$og_stage"' EXIT
mkdir -p "$og_stage/input" "$og_stage/output" "$og_output"
ditto "$og_root/target/aax-i386/OpenGridder.aaxplugin" "$og_stage/input/OpenGridder.aaxplugin"
lipo "$og_stage/input/OpenGridder.aaxplugin/Contents/MacOS/OpenGridder" -verify_arch i386
codesign --force --deep --timestamp --sign "$og_identity" "$og_stage/input/OpenGridder.aaxplugin"
"$og_wraptool" sign --verbose --account "${OPENGRIDDER_PACE_ACCOUNT:-mistamin}" \
  --wcguid "${OPENGRIDDER_PACE_GUID:-807F4D50-CFDB-11F1-A0CD-00505692AD3E}" \
  --dsig1-compat on --signid "$og_identity" --autoinstall on \
  --in "$og_stage/input/OpenGridder.aaxplugin" --out "$og_stage/output/OpenGridder.aaxplugin"
codesign --force --deep --timestamp --sign "$og_identity" "$og_stage/output/OpenGridder.aaxplugin"
codesign --verify --deep --strict --verbose=2 "$og_stage/output/OpenGridder.aaxplugin"
"$og_wraptool" verify --verbose --in "$og_stage/output/OpenGridder.aaxplugin" > "$og_output/pace-verification.txt"
lipo "$og_stage/output/OpenGridder.aaxplugin/Contents/MacOS/OpenGridder" -verify_arch i386
ditto "$og_stage/output/OpenGridder.aaxplugin" "$og_output/OpenGridder.aaxplugin"
echo 'Signed experimental i386 AAX; real Pro Tools 10 qualification remains required.'
