#!/usr/bin/env bash
set -euo pipefail
og_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
og_rust="${OG_LEGACY_RUSTC:-$HOME/.rustup/toolchains/1.41.1-x86_64-apple-darwin/bin/rustc}"
og_destination="$og_root/target/legacy-core-i686"
test -x "$og_rust"
mkdir -p "$og_destination"
/usr/bin/arch -x86_64 "$og_rust" --crate-name opengridder_legacy_bridge_core \
    --crate-type staticlib --edition=2018 --target i686-apple-darwin \
    -C panic=abort -C opt-level=2 "$og_root/native/legacy/bridge-core/src/lib.rs" \
    -o "$og_destination/libopengridder_legacy_bridge_core.a"
lipo "$og_destination/libopengridder_legacy_bridge_core.a" -verify_arch i386
shasum -a 256 "$og_destination/libopengridder_legacy_bridge_core.a" > "$og_destination/SHA256SUMS"
echo "i386 bridge primitives built. This archive is not an RTAS/TDM plugin or full network client."
