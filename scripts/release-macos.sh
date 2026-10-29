#!/bin/bash
# Builds every macOS format and the installer for one build class.
#   scripts/release-macos.sh modern   macOS 11.0 or later (both slices at 11.0)
#   scripts/release-macos.sh legacy   Intel back to macOS 10.12 (arm64 slice 11.0)
# The class and minimum are stamped into every bundle and checked by tools/opengrid_manifest.py.
# Needs AAX_SDK_ROOT and OPENGRIDDER_VST3_SDK; OG_SKIP_NOTARIZE=1 leaves the installers un-notarized.
set -uo pipefail
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
class="${1:?modern or legacy}"
case "$class" in
  modern) export OG_INTEL_MIN=11.0 OG_MIN_OS=11.0 ;;
  legacy) export OG_INTEL_MIN=10.12 OG_MIN_OS=10.12 ;;
  *) echo "usage: $0 modern|legacy" >&2; exit 2 ;;
esac
export OG_BUILD_CLASS="$class"
: "${AAX_SDK_ROOT:?set AAX_SDK_ROOT}" "${OPENGRIDDER_VST3_SDK:?set OPENGRIDDER_VST3_SDK}"
version="$(grep -m1 '^og_version=' scripts/package-release-macos.sh | cut -d'"' -f2)"
log="target/relbuild/$class"; mkdir -p "$log"
step() { local n=$1; shift; echo "== $n $(date +%T)"; if "$@" > "$log/$n.log" 2>&1; then echo "OK $n"; else echo "FAIL $n (see $log/$n.log)"; exit 1; fi; }

# AU and VST 2 without the old 32-bit slices: arm64 at 11.0, Intel at OG_INTEL_MIN.
slices() {
  f=$1
  cmake -S native/$f -B target/$f-arm64 -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=arm64 -DCMAKE_OSX_DEPLOYMENT_TARGET=11.0 -DOPENGRIDDER_RUST_LIBRARY="$PWD/target/release/libog_plugin.a" &&
  cmake --build target/$f-arm64 -j4 &&
  cmake -S native/$f -B target/$f-x86_64 -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=x86_64 -DCMAKE_OSX_DEPLOYMENT_TARGET="$OG_INTEL_MIN" -DCMAKE_CXX_COMPILER=/usr/bin/clang++ '-DCMAKE_CXX_COMPILER_LAUNCHER=/usr/bin/arch;-x86_64' -DOPENGRIDDER_RUST_LIBRARY="$PWD/target/rosetta-intel/x86_64-apple-darwin/release/libog_plugin.a" &&
  cmake --build target/$f-x86_64 -j4
}
universal() {
  d=$1; U=target/$d-universal; N=$([ "$d" = au ] && echo OpenGridder.component || echo OpenGridder.vst)
  rm -rf "$U" && mkdir -p "$U" && cp -R "target/$d-arm64/$N" "$U/$N" &&
  lipo -create "target/$d-x86_64/$N/Contents/MacOS/OpenGridder" "target/$d-arm64/$N/Contents/MacOS/OpenGridder" -output "$U/$N/Contents/MacOS/OpenGridder"
}
step aax bash scripts/build-aax-macos.sh
step au slices au
step vst2 slices vst2
step auv3 bash scripts/build-auv3-macos.sh
step vst3 bash scripts/build-vst3-macos.sh
step clap bash scripts/build-clap-macos.sh
step lv2 bash scripts/build-lv2-macos.sh
step server bash scripts/build-server-app-macos.sh
step win7 bash server/scripts/build-server-win7.sh
step universal-au universal au
step universal-vst2 universal vst2
cp target/au-rsrc/OpenGridder.rsrc target/au-universal/OpenGridder.component/Contents/Resources/
step signaax bash scripts/sign-aax-macos.sh
rm -rf target/release-licenses
step licenses python3 tools/collect_licenses.py --output target/release-licenses --aax-sdk "$AAX_SDK_ROOT" --vst3-sdk "$OPENGRIDDER_VST3_SDK"
variant=$([ "$class" = legacy ] && echo -legacy)
mkdir -p "dist/$version"
rm -f "dist/$version/OpenGridder-$version-macos$variant.pkg" "dist/$version/OpenGridder-$version-macos$variant.dmg"
step package bash scripts/package-release-macos.sh
echo "DONE $class"
