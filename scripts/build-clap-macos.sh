#!/usr/bin/env bash
set -euo pipefail

og_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
og_arm_tools="${OG_ARM_TOOLCHAIN:-$HOME/.rustup/toolchains/1.98.0-aarch64-apple-darwin}"
og_intel_tools="${OG_INTEL_TOOLCHAIN:-$HOME/.rustup/toolchains/1.98.0-x86_64-apple-darwin}"

export CARGO_PROFILE_RELEASE_STRIP=none

RUSTC="$og_arm_tools/bin/rustc" "$og_arm_tools/bin/cargo" build \
  --manifest-path "$og_root/Cargo.toml" --release --locked -p og-plugin \
  --target-dir "$og_root/target"

RUSTC="$og_intel_tools/bin/rustc" /usr/bin/arch -x86_64 "$og_intel_tools/bin/cargo" build \
  --manifest-path "$og_root/Cargo.toml" --release --locked -p og-plugin \
  --target x86_64-apple-darwin --target-dir "$og_root/target/rosetta-intel"

mkdir -p "$og_root/target/clap-arm64" "$og_root/target/clap-x86_64"

/usr/bin/clang++ -std=c++17 -O2 -fPIC -fobjc-arc -bundle \
  -arch arm64 -mmacosx-version-min=11.0 \
  -I"$og_root/native/include" \
  "$og_root/native/clap/OpenGridderCLAP.mm" \
  "$og_root/target/release/libog_plugin.a" \
  -framework Cocoa -framework QuartzCore -framework AudioToolbox -framework CoreFoundation -framework SystemConfiguration -framework Security \
  -o "$og_root/target/clap-arm64/OpenGridder"

/usr/bin/arch -x86_64 /usr/bin/clang++ -std=c++17 -O2 -fPIC -fobjc-arc -bundle \
  -arch x86_64 -mmacosx-version-min="${OG_INTEL_MIN:-10.13}" \
  -I"$og_root/native/include" \
  "$og_root/native/clap/OpenGridderCLAP.mm" \
  "$og_root/target/rosetta-intel/x86_64-apple-darwin/release/libog_plugin.a" \
  -framework Cocoa -framework QuartzCore -framework AudioToolbox -framework CoreFoundation -framework SystemConfiguration -framework Security \
  -o "$og_root/target/clap-x86_64/OpenGridder"

og_bundle="$og_root/target/clap-universal/OpenGridder.clap"
rm -rf "$og_bundle"
mkdir -p "$og_bundle/Contents/MacOS"
cp "$og_root/native/clap/Info.plist" "$og_bundle/Contents/Info.plist"

lipo -create \
  "$og_root/target/clap-x86_64/OpenGridder" \
  "$og_root/target/clap-arm64/OpenGridder" \
  -output "$og_bundle/Contents/MacOS/OpenGridder"

lipo "$og_bundle/Contents/MacOS/OpenGridder" -verify_arch x86_64 arm64
codesign --force --sign - --timestamp=none "$og_bundle"
codesign --verify --strict "$og_bundle"
echo "$og_bundle"
