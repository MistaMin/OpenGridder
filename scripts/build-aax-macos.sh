#!/usr/bin/env bash
set -euo pipefail

og_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
: "${AAX_SDK_ROOT:?Set AAX_SDK_ROOT to your externally installed AAX SDK}"
og_arm_tools="${OG_ARM_TOOLCHAIN:-$HOME/.rustup/toolchains/1.98.0-aarch64-apple-darwin}"
og_intel_tools="${OG_INTEL_TOOLCHAIN:-$HOME/.rustup/toolchains/1.98.0-x86_64-apple-darwin}"
for og_tools in "$og_arm_tools" "$og_intel_tools"; do
    test -x "$og_tools/bin/cargo" && test -x "$og_tools/bin/rustc" || {
        echo "Missing Rust toolchain: $og_tools" >&2; exit 1;
    }
done
export CARGO_PROFILE_RELEASE_STRIP=none

RUSTC="$og_arm_tools/bin/rustc" "$og_arm_tools/bin/cargo" build \
    --manifest-path "$og_root/Cargo.toml" --release --locked -p og-plugin \
    --target-dir "$og_root/target"
RUSTC="$og_intel_tools/bin/rustc" /usr/bin/arch -x86_64 "$og_intel_tools/bin/cargo" build \
    --manifest-path "$og_root/Cargo.toml" --release --locked -p og-plugin \
    --target x86_64-apple-darwin --target-dir "$og_root/target/rosetta-intel"

cmake -S "$og_root/native/aax" -B "$og_root/target/aax-arm64" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=arm64 \
    -DCMAKE_OSX_DEPLOYMENT_TARGET=11.0 -DAAX_SDK_ROOT="$AAX_SDK_ROOT" \
    -DOPENGRIDDER_RUST_LIBRARY="$og_root/target/release/libog_plugin.a"
cmake --build "$og_root/target/aax-arm64" -j "${OG_BUILD_JOBS:-4}"
cmake -S "$og_root/native/aax" -B "$og_root/target/aax-intel-rosetta" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=x86_64 \
    -DCMAKE_OSX_DEPLOYMENT_TARGET="${OG_INTEL_MIN:-10.12}" -DCMAKE_CXX_COMPILER=/usr/bin/clang++ \
    '-DCMAKE_CXX_COMPILER_LAUNCHER=/usr/bin/arch;-x86_64' \
    -DAAX_SDK_ROOT="$AAX_SDK_ROOT" \
    -DOPENGRIDDER_RUST_LIBRARY="$og_root/target/rosetta-intel/x86_64-apple-darwin/release/libog_plugin.a"
cmake --build "$og_root/target/aax-intel-rosetta" -j "${OG_BUILD_JOBS:-4}"

og_bundle="$og_root/target/aax-universal/OpenGridder.aaxplugin"
cmake -E copy_directory "$og_root/target/aax-arm64/OpenGridder.aaxplugin" "$og_bundle"
lipo -create "$og_root/target/aax-arm64/OpenGridder.aaxplugin/Contents/MacOS/OpenGridder" \
    "$og_root/target/aax-intel-rosetta/OpenGridder.aaxplugin/Contents/MacOS/OpenGridder" \
    -output "$og_bundle/Contents/MacOS/OpenGridder"
lipo "$og_bundle/Contents/MacOS/OpenGridder" -verify_arch arm64 x86_64
codesign --force --sign - --timestamp=none "$og_bundle"
codesign --verify --strict "$og_bundle"
echo "Development AAX bundle: $og_bundle"
echo "Ad-hoc signature only. PACE signing is required for production Pro Tools."
