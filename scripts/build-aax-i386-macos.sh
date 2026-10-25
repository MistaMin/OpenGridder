#!/bin/bash
# PT10 target. Requires independently installed Apple SDK and matching C++ headers.
# Never merges this experimental slice into the qualified 64-bit alpha artifact.
set -euo pipefail
og_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
: "${AAX_SDK_ROOT:?Set the external AAX SDK path}"
: "${OPENGRIDDER_LEGACY_APPLE_SDK:?Provide a real legacy Apple SDK with i386 system/framework libraries}"
: "${OPENGRIDDER_LEGACY_CXX_HEADERS:?Provide matching legacy libc++ v1 headers from the archived Xcode}"
test -f "$OPENGRIDDER_LEGACY_CXX_HEADERS/vector"
test -d "$OPENGRIDDER_LEGACY_APPLE_SDK/System/Library/Frameworks/CoreFoundation.framework"
og_system="$OPENGRIDDER_LEGACY_APPLE_SDK/usr/lib/libSystem.dylib"
if [ -f "$og_system" ]; then
    lipo "$og_system" -verify_arch i386
else
    python3 - "$OPENGRIDDER_LEGACY_APPLE_SDK/usr/lib/libSystem.tbd" <<'PY'
import sys
from pathlib import Path
if 'i386' not in Path(sys.argv[1]).read_text(): raise SystemExit('Apple SDK lacks i386 libSystem')
PY
fi
# Xcode 4.6 libc++ places [[noreturn]] before friend, which modern Clang rejects.
# Patch a generated header copy only; preserve the independently installed SDK.
og_cxx_headers="$og_root/target/legacy-cxx-headers"
python3 - "$OPENGRIDDER_LEGACY_CXX_HEADERS" "$og_cxx_headers" <<'PYHEADERS'
import shutil,sys
from pathlib import Path
source,target=map(Path,sys.argv[1:])
shutil.copytree(source,target,dirs_exist_ok=True,symlinks=True)
config=target/'__config'
text=config.read_text()
text=text.replace('#  define _LIBCPP_NORETURN [[noreturn]]',
                  '#  define _LIBCPP_NORETURN __attribute__ ((noreturn))')
config.write_text(text)
PYHEADERS
bash "$og_root/scripts/build-legacy-client-macos.sh"
og_build="$og_root/target/aax-i386"
cmake -S "$og_root/native/aax" -B "$og_build" -G Ninja \
    -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=i386 \
    -DCMAKE_OSX_DEPLOYMENT_TARGET=10.8 -DCMAKE_TRY_COMPILE_TARGET_TYPE=STATIC_LIBRARY \
    -DCMAKE_OSX_SYSROOT="$OPENGRIDDER_LEGACY_APPLE_SDK" \
    "-DCMAKE_CXX_FLAGS=-nostdinc++ -isystem \"$og_cxx_headers\"" \
    -DAAX_SDK_ROOT="$AAX_SDK_ROOT" \
    -DOPENGRIDDER_RUST_LIBRARY="$og_root/target/legacy-client-i686/libopengridder_legacy_client.a"
cmake --build "$og_build" -j "${OG_BUILD_JOBS:-4}"
og_bundle="$og_build/OpenGridder.aaxplugin"
lipo "$og_bundle/Contents/MacOS/OpenGridder" -verify_arch i386
nm -g "$og_bundle/Contents/MacOS/OpenGridder" > "$og_build/symbols.txt"
python3 - "$og_build/symbols.txt" <<'PY'
from pathlib import Path
import re,sys
text=Path(sys.argv[1]).read_text()
for name in ('ACFRegisterPlugin','ACFRegisterComponent','ACFGetClassFactory'):
    if not re.search(r' T _'+name+r'$',text,re.M): raise SystemExit('Missing AAX export: '+name)
PY
echo "Experimental PT10 AAX bundle: $og_bundle"
echo 'Not PACE-signed or host-qualified. No compatibility claim before actual PT10 validation.'
