#!/usr/bin/env bash
# Universal AUv3 build script for modern macOS machines (arm64 + x86_64).
set -euo pipefail

og_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

og_arm_tools="${OG_ARM_TOOLCHAIN:-$HOME/.rustup/toolchains/1.98.0-aarch64-apple-darwin}"
og_intel_tools="${OG_INTEL_TOOLCHAIN:-$HOME/.rustup/toolchains/1.98.0-x86_64-apple-darwin}"

echo "=== 1. Building Rust client libraries ==="
export CARGO_PROFILE_RELEASE_STRIP=none

RUSTC="$og_arm_tools/bin/rustc" "$og_arm_tools/bin/cargo" build \
    --manifest-path "$og_root/Cargo.toml" --release --locked -p og-plugin \
    --target-dir "$og_root/target"

RUSTC="$og_intel_tools/bin/rustc" /usr/bin/arch -x86_64 "$og_intel_tools/bin/cargo" build \
    --manifest-path "$og_root/Cargo.toml" --release --locked -p og-plugin \
    --target x86_64-apple-darwin --target-dir "$og_root/target/rosetta-intel"

echo "=== 2. Compiling AUv3 Slices ==="
# ARM64 slice
cmake -S "$og_root/native/auv3" -B "$og_root/target/auv3-arm64" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=arm64 \
    -DCMAKE_OSX_DEPLOYMENT_TARGET=11.0 \
    -DOPENGRIDDER_RUST_LIBRARY="$og_root/target/release/libog_plugin.a"
cmake --build "$og_root/target/auv3-arm64" -j "${OG_BUILD_JOBS:-4}"

# x86_64 slice
cmake -S "$og_root/native/auv3" -B "$og_root/target/auv3-x86_64" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=x86_64 \
    -DCMAKE_OSX_DEPLOYMENT_TARGET="${OG_INTEL_MIN:-10.15}" \
    -DOPENGRIDDER_RUST_LIBRARY="$og_root/target/rosetta-intel/x86_64-apple-darwin/release/libog_plugin.a"
cmake --build "$og_root/target/auv3-x86_64" -j "${OG_BUILD_JOBS:-4}"

echo "=== 3. Assembling Universal OpenGridderAUv3.appex ==="
og_auv3_dest="$og_root/target/auv3-universal/OpenGridderAUv3.appex"
rm -rf "$og_auv3_dest"
mkdir -p "$og_auv3_dest/Contents/MacOS"
ditto "$og_root/target/auv3-arm64/OpenGridderAUv3.appex" "$og_auv3_dest"

lipo -create \
    "$og_root/target/auv3-arm64/OpenGridderAUv3.appex/Contents/MacOS/OpenGridderAUv3" \
    "$og_root/target/auv3-x86_64/OpenGridderAUv3.appex/Contents/MacOS/OpenGridderAUv3" \
    -output "$og_auv3_dest/Contents/MacOS/OpenGridderAUv3"

lipo -info "$og_auv3_dest/Contents/MacOS/OpenGridderAUv3"
echo "SUCCESS: Built Universal AUv3 at $og_auv3_dest"
