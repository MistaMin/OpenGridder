#!/usr/bin/env bash
# Universal AUv2 build script for Logic 8, Logic 9 (32/64-bit), and modern DAWs.
set -euo pipefail

og_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
: "${OPENGRIDDER_RTAS_APPLE_SDK:?Set Apple 10.6 SDK path for Rez and Component Manager headers}"
: "${OPENGRIDDER_LEGACY_APPLE_SDK:?Set Apple 10.8 SDK path with i386 libraries}"
: "${OPENGRIDDER_LEGACY_CXX_HEADERS:?Set archived Xcode libc++ headers}"

og_arm_tools="${OG_ARM_TOOLCHAIN:-$HOME/.rustup/toolchains/1.98.0-aarch64-apple-darwin}"
og_intel_tools="${OG_INTEL_TOOLCHAIN:-$HOME/.rustup/toolchains/1.98.0-x86_64-apple-darwin}"

echo "=== 1. Building Rust client libraries ==="
export CARGO_PROFILE_RELEASE_STRIP=none

# ARM64 & x86_64 modern plugins
RUSTC="$og_arm_tools/bin/rustc" "$og_arm_tools/bin/cargo" build \
    --manifest-path "$og_root/Cargo.toml" --release --locked -p og-plugin \
    --target-dir "$og_root/target"

RUSTC="$og_intel_tools/bin/rustc" /usr/bin/arch -x86_64 "$og_intel_tools/bin/cargo" build \
    --manifest-path "$og_root/Cargo.toml" --release --locked -p og-plugin \
    --target x86_64-apple-darwin --target-dir "$og_root/target/rosetta-intel"

# i386 legacy client
bash "$og_root/scripts/build-legacy-client-macos.sh"

echo "=== 2. Compiling Component Manager 'thng' resource for Logic 8 ==="
og_rsrc_dir="$og_root/target/au-rsrc"
mkdir -p "$og_rsrc_dir"
/usr/bin/rez -useDF \
  -isysroot "$OPENGRIDDER_RTAS_APPLE_SDK" \
  -i "$OPENGRIDDER_RTAS_APPLE_SDK/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/CarbonCore.framework/Versions/A/Headers" \
  -o "$og_rsrc_dir/OpenGridder.rsrc" \
  "$og_root/native/au/OpenGridderAU.r"
test -s "$og_rsrc_dir/OpenGridder.rsrc"

echo "=== 3. Compiling C++ AU Slices ==="
# Setup patched libc++ headers for legacy i386 build
og_cxx_headers="$og_root/target/legacy-cxx-headers"
python3 - "$OPENGRIDDER_LEGACY_CXX_HEADERS" "$og_cxx_headers" <<'PYHEADERS'
import shutil,sys
from pathlib import Path
source,target=map(Path,sys.argv[1:])
if source.resolve() != target.resolve():
    shutil.copytree(source,target,dirs_exist_ok=True,symlinks=True)
config=target/'__config'
text=config.read_text()
text=text.replace('#  define _LIBCPP_NORETURN [[noreturn]]',
                  '#  define _LIBCPP_NORETURN __attribute__ ((noreturn))')
config.write_text(text)
PYHEADERS

# ARM64 slice
cmake -S "$og_root/native/au" -B "$og_root/target/au-arm64" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=arm64 \
    -DCMAKE_OSX_DEPLOYMENT_TARGET=11.0 \
    -DOPENGRIDDER_RUST_LIBRARY="$og_root/target/release/libog_plugin.a"
cmake --build "$og_root/target/au-arm64" -j "${OG_BUILD_JOBS:-4}"

# x86_64 slice (Logic 9 64-bit & Intel Macs)
cmake -S "$og_root/native/au" -B "$og_root/target/au-x86_64" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=x86_64 \
    -DCMAKE_OSX_DEPLOYMENT_TARGET="${OG_INTEL_MIN:-10.12}" -DCMAKE_CXX_COMPILER=/usr/bin/clang++ \
    '-DCMAKE_CXX_COMPILER_LAUNCHER=/usr/bin/arch;-x86_64' \
    -DOPENGRIDDER_RUST_LIBRARY="$og_root/target/rosetta-intel/x86_64-apple-darwin/release/libog_plugin.a"
cmake --build "$og_root/target/au-x86_64" -j "${OG_BUILD_JOBS:-4}"

# i386 slice (Logic 8, 32-bit Logic 9, OS X 10.6-10.8)
cmake -S "$og_root/native/au" -B "$og_root/target/au-i386" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=i386 \
    -DCMAKE_OSX_DEPLOYMENT_TARGET=10.8 -DCMAKE_TRY_COMPILE_TARGET_TYPE=STATIC_LIBRARY \
    -DCMAKE_OSX_SYSROOT="$OPENGRIDDER_LEGACY_APPLE_SDK" \
    "-DCMAKE_CXX_FLAGS=-nostdinc++ -isystem \"$og_cxx_headers\" -I\"$OPENGRIDDER_RTAS_APPLE_SDK/System/Library/Frameworks/CoreServices.framework/Versions/A/Frameworks/CarbonCore.framework/Versions/A/Headers\"" \
    -DOPENGRIDDER_RUST_LIBRARY="$og_root/target/legacy-client-i686/libopengridder_legacy_client.a"
cmake --build "$og_root/target/au-i386" -j "${OG_BUILD_JOBS:-4}"

echo "=== 4. Assembling Universal OpenGridder.component ==="
og_bundle="$og_root/target/au-universal/OpenGridder.component"
rm -rf "$og_bundle"
mkdir -p "$(dirname "$og_bundle")"
cmake -E copy_directory "$og_root/target/au-arm64/OpenGridder.component" "$og_bundle"

# Lipo create tri-architecture universal binary (i386, x86_64, arm64)
lipo -create \
    "$og_root/target/au-i386/OpenGridder.component/Contents/MacOS/OpenGridder" \
    "$og_root/target/au-x86_64/OpenGridder.component/Contents/MacOS/OpenGridder" \
    "$og_root/target/au-arm64/OpenGridder.component/Contents/MacOS/OpenGridder" \
    -output "$og_bundle/Contents/MacOS/OpenGridder"

# Embed Logic 8 'thng' resource file
cp "$og_rsrc_dir/OpenGridder.rsrc" "$og_bundle/Contents/Resources/OpenGridder.rsrc"

# Verify architectures
lipo "$og_bundle/Contents/MacOS/OpenGridder" -verify_arch i386 x86_64 arm64
echo "Architectures verified: i386 x86_64 arm64"

# Verify entry points in symbol table
nm -arch i386 -g "$og_bundle/Contents/MacOS/OpenGridder" | grep "T _OpenGridderAUEntry"
nm -arch all -g "$og_bundle/Contents/MacOS/OpenGridder" | grep "T _OpenGridderAUFactory"
echo "Entry points verified: _OpenGridderAUEntry (Component Manager i386) & _OpenGridderAUFactory (AudioComponent Universal)"

# Sign bundle (ad-hoc or developer identity)
if [ -n "${OPENGRIDDER_SIGN_IDENTITY:-}" ]; then
  codesign --force --deep --timestamp --digest-algorithm=sha1,sha256 \
    --sign "$OPENGRIDDER_SIGN_IDENTITY" "$og_bundle"
  codesign --verify --deep --strict --verbose=2 "$og_bundle"
else
  codesign --force --sign - --timestamp=none "$og_bundle"
  codesign --verify --strict "$og_bundle"
fi

echo "Successfully built Universal Logic 8/9 AudioUnit:"
echo "$og_bundle"
