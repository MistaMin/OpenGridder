#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

OUTPUT_DIR="${REPO_ROOT}/target/server-windows7"
mkdir -p "${OUTPUT_DIR}"

echo "=== [1/4] Checking toolchains ==="
if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
    echo "ERROR: x86_64-w64-mingw32-gcc not found. Please install mingw-w64."
    exit 1
fi

rustup target add x86_64-pc-windows-gnu 2>/dev/null || true

echo "=== [2/4] Building Win7 forwarder compatibility DLLs ==="
x86_64-w64-mingw32-gcc -O2 -shared -s \
    "${REPO_ROOT}/native/win7compat/bcryptprimitives.c" \
    -o "${OUTPUT_DIR}/bcryptprimitives.dll" \
    -Wl,--subsystem,windows:6.1

x86_64-w64-mingw32-gcc -O2 -shared -s \
    "${REPO_ROOT}/native/win7compat/synch.c" \
    -o "${OUTPUT_DIR}/api-ms-win-core-synch-l1-2-0.dll" \
    -Wl,--subsystem,windows:6.1

echo "=== [3/4] Compiling OpenGridder Windows Server binary ==="
cargo build --target x86_64-pc-windows-gnu --bin AudioGridderServer --release

cp "${REPO_ROOT}/target/x86_64-pc-windows-gnu/release/AudioGridderServer.exe" "${OUTPUT_DIR}/"

echo "=== [4/4] Writing Windows 7 deployment instructions ==="
cat << 'EOF' > "${OUTPUT_DIR}/README-Windows7.txt"
OpenGridder Server for Windows 7 SP1 (64-bit)
==============================================

Compatibility:
- Tested for Windows 7 SP1 (NT 6.1) and Windows Server 2008 R2.
- Backwards compatible through Windows 8, 8.1, 10, and 11.
- Required Windows 7 Updates: KB2999226 (Universal CRT) or VC++ 2015-2022 Redistributable.

Files included:
- AudioGridderServer.exe: OpenGridder pure-Rust headless server.
- api-ms-win-core-synch-l1-2-0.dll: Win7 compatibility shim for WaitOnAddress/WakeByAddress.
- bcryptprimitives.dll: Win7 compatibility shim routing ProcessPrng to Advapi32 RtlGenRandom.

Usage:
1. Extract or place all files in the same directory (e.g. C:\OpenGridder\).
   DO NOT remove the included .dll files on Windows 7!
2. Run from Command Prompt:
   AudioGridderServer.exe --bind 0.0.0.0
3. Ensure TCP ports 55056+ are allowed through Windows Firewall.
EOF

echo "SUCCESS: Windows 7 server package created at ${OUTPUT_DIR}"
ls -lh "${OUTPUT_DIR}"
