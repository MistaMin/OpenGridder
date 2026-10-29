#!/bin/bash
# Runs the checks of .github/workflows/ci.yml on this Mac, for when hosted CI is not available.
# macOS only: the Ubuntu and Intel-runner legs of CI are not covered (the legacy tests do run under Rosetta).
set -euo pipefail
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export OPENGRIDDER_VST3_SDK="${OPENGRIDDER_VST3_SDK:?Set OPENGRIDDER_VST3_SDK to the VST3 SDK}"
echo "== workspace tests";            cargo test --workspace --locked
echo "== server workspace tests";     (cd server && cargo test --workspace --locked)
echo "== legacy bridge tests";        cargo test --lib --manifest-path native/legacy/bridge-core/Cargo.toml
echo "== SDK-free server build";       (cd server && cargo build --release --locked -p og-server --bins)
echo "== legacy client: i386 archive, unit tests, real server"
OG_LEGACY_RUN_TESTS=1 OPENGRIDDER_TEST_SERVER="$PWD/server/target/release/OpenGridderServer" bash scripts/build-legacy-client-macos.sh
echo "== legacy server (builds natively, checks audio, parameters and the streamed editor)"
(cd server && bash legacy-server/scripts/test-legacy-server.sh)
echo "== build class flag (tools/opengrid_manifest.py)"
python3 tools/test_opengrid_manifest.py
for release in dist/*/opengrid-manifest.json; do [ -f "$release" ] && python3 tools/opengrid_manifest.py verify "$(dirname "$release")"; done
echo "ALL LOCAL CI CHECKS PASSED"
