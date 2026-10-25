#!/usr/bin/env bash
# Universal VST3 build script for modern macOS DAWs (arm64 + x86_64).
set -euo pipefail

og_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
og_arm_tools="${OG_ARM_TOOLCHAIN:-$HOME/.rustup/toolchains/1.98.0-aarch64-apple-darwin}"
og_intel_tools="${OG_INTEL_TOOLCHAIN:-$HOME/.rustup/toolchains/1.98.0-x86_64-apple-darwin}"

echo "=== 1. Building Rust Core Libraries ==="
RUSTC="$og_arm_tools/bin/rustc" "$og_arm_tools/bin/cargo" build \
    --manifest-path "$og_root/Cargo.toml" --release --locked -p og-plugin \
    --target-dir "$og_root/target"

RUSTC="$og_intel_tools/bin/rustc" /usr/bin/arch -x86_64 "$og_intel_tools/bin/cargo" build \
    --manifest-path "$og_root/Cargo.toml" --release --locked -p og-plugin \
    --target x86_64-apple-darwin --target-dir "$og_root/target/rosetta-intel"

echo "=== 2. Compiling C++ VST3 Slices ==="
# arm64 slice (Apple Silicon)
cmake -S "$og_root/native/vst3" -B "$og_root/target/vst3-arm64" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=arm64 \
    -DCMAKE_OSX_DEPLOYMENT_TARGET=11.0 \
    -DOPENGRIDDER_RUST_LIBRARY="$og_root/target/release/libog_plugin.a"
cmake --build "$og_root/target/vst3-arm64" -j "${OG_BUILD_JOBS:-4}"

# x86_64 slice (Intel)
cmake -S "$og_root/native/vst3" -B "$og_root/target/vst3-x86_64" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=x86_64 \
    -DCMAKE_OSX_DEPLOYMENT_TARGET=10.12 -DCMAKE_CXX_COMPILER=/usr/bin/clang++ \
    '-DCMAKE_CXX_COMPILER_LAUNCHER=/usr/bin/arch;-x86_64' \
    -DOPENGRIDDER_RUST_LIBRARY="$og_root/target/rosetta-intel/x86_64-apple-darwin/release/libog_plugin.a"
cmake --build "$og_root/target/vst3-x86_64" -j "${OG_BUILD_JOBS:-4}"

echo "=== 3. Assembling Universal OpenGridder.vst3 ==="
og_bundle="$og_root/target/vst3-universal/OpenGridder.vst3"
rm -rf "$og_bundle"
mkdir -p "$(dirname "$og_bundle")"
cmake -E copy_directory "$og_root/target/vst3-arm64/OpenGridder.vst3" "$og_bundle"

# Lipo universal binary (arm64 + x86_64)
lipo -create \
    "$og_root/target/vst3-x86_64/OpenGridder.vst3/Contents/MacOS/OpenGridder" \
    "$og_root/target/vst3-arm64/OpenGridder.vst3/Contents/MacOS/OpenGridder" \
    -output "$og_bundle/Contents/MacOS/OpenGridder"

# Verify architectures
lipo "$og_bundle/Contents/MacOS/OpenGridder" -verify_arch arm64 x86_64
echo "Architectures verified: arm64 x86_64"

# Verify GetPluginFactory export
nm -arch all -g "$og_bundle/Contents/MacOS/OpenGridder" | grep "T _GetPluginFactory"
echo "Export verified: _GetPluginFactory"

echo "SUCCESS: Built Universal VST3 at $og_bundle"
