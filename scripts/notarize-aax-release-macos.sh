#!/bin/bash
# Submit generated preview packages to Apple; never publish to GitHub here.
set -euo pipefail
og_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
og_version="1.2.1-preview.1"
og_dist="$og_root/dist/$og_version"
og_stage="$og_root/target/release-stage/$og_version/dmg"
og_profile="${OPENGRIDDER_NOTARY_PROFILE:-AC_NOTARY}"
og_identity="${OPENGRIDDER_SIGN_IDENTITY:-Developer ID Application: Marcos Deida (ZQYU67JUBH)}"
og_pkg="$og_dist/OpenGridder-$og_version-macos.pkg"
og_dmg="$og_dist/OpenGridder-$og_version-macos.dmg"
test -f "$og_pkg"
xcrun notarytool submit "$og_pkg" --keychain-profile "$og_profile" --wait --output-format json > "$og_dist/pkg-notary.json"
python3 - "$og_dist/pkg-notary.json" <<'PY'
import json,sys
r=json.load(open(sys.argv[1])); print(r)
if r.get('status') != 'Accepted': raise SystemExit('Installer notarization was not accepted')
PY
xcrun stapler staple "$og_pkg"
xcrun stapler validate "$og_pkg"
/usr/sbin/spctl --assess --type install --verbose=2 "$og_pkg"
ditto "$og_pkg" "$og_stage/$(basename "$og_pkg")"
hdiutil create -volname "OpenGridder AAX Preview" -srcfolder "$og_stage" -ov -format UDZO "$og_dmg"
codesign --timestamp --sign "$og_identity" "$og_dmg"
codesign --verify --strict "$og_dmg"
xcrun notarytool submit "$og_dmg" --keychain-profile "$og_profile" --wait --output-format json > "$og_dist/dmg-notary.json"
python3 - "$og_dist/dmg-notary.json" <<'PY'
import json,sys
r=json.load(open(sys.argv[1])); print(r)
if r.get('status') != 'Accepted': raise SystemExit('Disk image notarization was not accepted')
PY
xcrun stapler staple "$og_dmg"
xcrun stapler validate "$og_dmg"
/usr/sbin/spctl --assess --type open --context context:primary-signature --verbose=2 "$og_dmg"
(cd "$og_dist" && shasum -a 256 "OpenGridder-$og_version-macos.pkg" "OpenGridder-$og_version-macos.dmg" > SHA256SUMS)
echo "Notarized preview assets prepared: $og_dist"
