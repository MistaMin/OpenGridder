#!/usr/bin/env bash
# Universal macOS Application builder for OpenGridder Server.app (arm64 + x86_64).
set -euo pipefail

og_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

og_server_bin_tmp="$og_root/target/server-universal/OpenGridderServer-bin"
og_existing_server="$og_root/target/server-universal/OpenGridderServer"

: "${OPENGRIDDER_VST3_SDK:=/Volumes/Developer/Dev Projects/Water Channel/third_party/vst3sdk}"
export OPENGRIDDER_VST3_SDK

echo "=== 1. Building Server Universal Rust Binary (VST3 SDK: $OPENGRIDDER_VST3_SDK) ==="
og_arm_tools="${OG_ARM_TOOLCHAIN:-$HOME/.rustup/toolchains/1.98.0-aarch64-apple-darwin}"
og_intel_tools="${OG_INTEL_TOOLCHAIN:-$HOME/.rustup/toolchains/1.98.0-x86_64-apple-darwin}"
export CARGO_PROFILE_RELEASE_STRIP=none
OG_BUILD_CLASS="${OG_BUILD_CLASS:-modern}" RUSTC="$og_arm_tools/bin/rustc" "$og_arm_tools/bin/cargo" build \
    --manifest-path "$og_root/server/Cargo.toml" --release --locked -p og-server --target-dir "$og_root/target"
# The deployment target is read when linking: make sure the program is linked again.
rm -f "$og_root/target/rosetta-intel/x86_64-apple-darwin/release/OpenGridderServer"
MACOSX_DEPLOYMENT_TARGET="${OG_INTEL_MIN:-10.12}" OG_BUILD_CLASS="${OG_BUILD_CLASS:-modern}" RUSTC="$og_intel_tools/bin/rustc" /usr/bin/arch -x86_64 "$og_intel_tools/bin/cargo" build \
    --manifest-path "$og_root/server/Cargo.toml" --release --locked -p og-server --target x86_64-apple-darwin \
    --target-dir "$og_root/target/rosetta-intel"
mkdir -p "$og_root/target/server-universal"
lipo -create "$og_root/target/release/OpenGridderServer" \
    "$og_root/target/rosetta-intel/x86_64-apple-darwin/release/OpenGridderServer" \
    -output "$og_server_bin_tmp"
cp "$og_server_bin_tmp" "$og_existing_server"
lipo "$og_server_bin_tmp" -verify_arch arm64 x86_64

echo "=== 2. Compiling Cocoa Launcher Slices ==="
mkdir -p "$og_root/target/server-app-build"
/usr/bin/clang -O3 -fobjc-arc -arch arm64 -mmacosx-version-min=11.0 \
    -framework Cocoa \
    -o "$og_root/target/server-app-build/launcher-arm64" \
    "$og_root/server/native/server_app/main.m"

/usr/bin/clang -O3 -fobjc-arc -arch x86_64 -mmacosx-version-min="${OG_INTEL_MIN:-10.15}" \
    -framework Cocoa \
    -o "$og_root/target/server-app-build/launcher-x86_64" \
    "$og_root/server/native/server_app/main.m"

lipo -create \
    "$og_root/target/server-app-build/launcher-arm64" \
    "$og_root/target/server-app-build/launcher-x86_64" \
    -output "$og_root/target/server-app-build/OpenGridderServer"

echo "=== 3. Assembling OpenGridder Server.app ==="
og_app_dest="$og_root/target/server-app/OpenGridder Server.app"
rm -rf "$og_app_dest"
mkdir -p "$og_app_dest/Contents/MacOS"
mkdir -p "$og_app_dest/Contents/Resources"
mkdir -p "$og_app_dest/Contents/PlugIns"

cp "$og_root/target/server-app-build/OpenGridderServer" "$og_app_dest/Contents/MacOS/OpenGridderServer"
chmod 755 "$og_app_dest/Contents/MacOS/OpenGridderServer"

cp "$og_server_bin_tmp" "$og_app_dest/Contents/MacOS/OpenGridderServer-bin"
chmod 755 "$og_app_dest/Contents/MacOS/OpenGridderServer-bin"

cp "$og_root/server/native/server_app/Info.plist.in" "$og_app_dest/Contents/Info.plist"
printf 'APPL????' > "$og_app_dest/Contents/PkgInfo"

if [ -f "$og_root/server/Resources/OpenGridderServer.icns" ]; then
    cp "$og_root/server/Resources/OpenGridderServer.icns" "$og_app_dest/Contents/Resources/OpenGridderServer.icns"
fi

# Embed AUv3 App Extension if built
if [ -d "$og_root/target/auv3-universal/OpenGridderAUv3.appex" ]; then
    ditto "$og_root/target/auv3-universal/OpenGridderAUv3.appex" "$og_app_dest/Contents/PlugIns/OpenGridderAUv3.appex"
fi

lipo -info "$og_app_dest/Contents/MacOS/OpenGridderServer"
lipo -info "$og_app_dest/Contents/MacOS/OpenGridderServer-bin"

echo "SUCCESS: Built OpenGridder Server.app at $og_app_dest"
