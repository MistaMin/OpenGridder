#!/usr/bin/env bash
set -euo pipefail
og_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
: "${OPENGRIDDER_VST3_SDK:?Set OPENGRIDDER_VST3_SDK to your external Steinberg SDK root}"
og_arm_tools="${OG_ARM_TOOLCHAIN:-$HOME/.rustup/toolchains/1.98.0-aarch64-apple-darwin}"
og_intel_tools="${OG_INTEL_TOOLCHAIN:-$HOME/.rustup/toolchains/1.98.0-x86_64-apple-darwin}"
export CARGO_PROFILE_RELEASE_STRIP=none
RUSTC="$og_arm_tools/bin/rustc" "$og_arm_tools/bin/cargo" build \
    --manifest-path "$og_root/server/Cargo.toml" --release --locked -p og-server --target-dir "$og_root/target"
RUSTC="$og_intel_tools/bin/rustc" /usr/bin/arch -x86_64 "$og_intel_tools/bin/cargo" build \
    --manifest-path "$og_root/server/Cargo.toml" --release --locked -p og-server --target x86_64-apple-darwin \
    --target-dir "$og_root/target/rosetta-intel"
og_destination="$og_root/target/server-universal"
mkdir -p "$og_destination"
lipo -create "$og_root/target/release/OpenGridderServer" \
    "$og_root/target/rosetta-intel/x86_64-apple-darwin/release/OpenGridderServer" \
    -output "$og_destination/OpenGridderServer"
lipo "$og_destination/OpenGridderServer" -verify_arch arm64 x86_64
codesign --force --sign - --timestamp=none "$og_destination/OpenGridderServer"
codesign --verify --strict "$og_destination/OpenGridderServer"
"$og_destination/OpenGridderServer" --version
/usr/bin/arch -x86_64 "$og_destination/OpenGridderServer" --version
shasum -a 256 "$og_destination/OpenGridderServer" > "$og_destination/SHA256SUMS"
echo "Development server: $og_destination/OpenGridderServer"
