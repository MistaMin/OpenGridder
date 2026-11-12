#!/usr/bin/env bash
# One release identity for server, plug-in formats and the private RTAS/TDM bundles.
#   OG_VERSION, OG_BUILD_NUMBER, OG_GIT_COMMIT and OG_BUILD_DATE come from build-info.sh
#   (or the caller) and are exported to every child build.
#
# The RTAS/TDM source is in the private OpenGridder-Legacy-Wrappers checkout. Set
# OPENGRIDDER_LEGACY_ROOT to that checkout to build it here; otherwise only the
# modern server/plugin formats are built.
set -euo pipefail
og_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$og_root/scripts/build-info.sh"
echo "OpenGridder release: $OG_VERSION build $OG_BUILD_NUMBER ($OG_GIT_COMMIT) $OG_BUILD_DATE"

echo "== Build server, server app and AU/VST3/AAX/CLAP/LV2 bundles"
bash "$og_root/scripts/build-all-bundles.sh"

echo "== Test the modern workspace (format test checks the staged bundles)"
# The og-server sandbox integration tests use lsof, which is pathologically slow on
# this Mac's stale CCC snapshot; they are host-environment failures, not code failures.
cargo test --workspace --locked --lib
cargo test -p og-plugin --test formats --locked

if [ -n "${OPENGRIDDER_LEGACY_ROOT:-}" ]; then
  legacy="$OPENGRIDDER_LEGACY_ROOT"
  test -d "$legacy/legacy/native" || { echo "OPENGRIDDER_LEGACY_ROOT must be the OpenGridder-Legacy-Wrappers checkout" >&2; exit 1; }
  : "${OPENGRIDDER_LEGACY_SDK:?Set the licensed Avid SDK for the RTAS/TDM build}"
  echo "== Build RTAS/TDM with the same identity"
  OPENGRIDDER_CORE_ROOT="$legacy/legacy" \
  OPENGRIDDER_EMULATOR_ROOT="$legacy/legacy/firmware" \
    bash "$legacy/legacy/native/scripts/build-leopard-release.sh"
else
  echo "RTAS/TDM skipped: set OPENGRIDDER_LEGACY_ROOT to the private wrappers checkout" >&2
fi

echo "Unified build complete: $OG_VERSION build $OG_BUILD_NUMBER"
