#!/usr/bin/env bash
# Universal macOS Application builder for OpenGridder Server.app
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
og_root="$(cd "$script_dir/.." && pwd)"
source "$script_dir/build-info.sh"
echo "Build: v$OG_VERSION build $OG_BUILD_NUMBER ($OG_GIT_COMMIT) $OG_BUILD_DATE"

echo "=== 1. Building Server Binaries ==="
cd "$og_root"
cargo build --release --locked -p ag-server -p ag-tray

og_server_bin="$og_root/target/release/ag-server"
og_app_dest="$og_root/build/OpenGridder Server.app"

echo "=== 2. Compiling Cocoa Launcher Slice ==="
mkdir -p "$og_root/target/server-app-build"
/usr/bin/clang -O3 -fobjc-arc -arch arm64 -mmacosx-version-min=11.0 \
    -framework Cocoa \
    -framework ApplicationServices \
    -o "$og_root/target/server-app-build/OpenGridderServer" \
    "$og_root/app/server/main.m"

echo "=== 3. Assembling OpenGridder Server.app ==="
rm -rf "$og_app_dest"
mkdir -p "$og_app_dest/Contents/MacOS"
mkdir -p "$og_app_dest/Contents/Resources"

cp "$og_root/target/server-app-build/OpenGridderServer" "$og_app_dest/Contents/MacOS/OpenGridderServer"
chmod 755 "$og_app_dest/Contents/MacOS/OpenGridderServer"

cp "$og_server_bin" "$og_app_dest/Contents/MacOS/OpenGridderServer-bin"
chmod 755 "$og_app_dest/Contents/MacOS/OpenGridderServer-bin"

sed -e "s|@OG_VERSION@|$OG_VERSION|g" \
    -e "s|@OG_BUILD_NUMBER@|$OG_BUILD_NUMBER|g" \
    -e "s|@OG_GIT_COMMIT@|$OG_GIT_COMMIT|g" \
    -e "s|@OG_BUILD_DATE@|$OG_BUILD_DATE|g" \
    "$og_root/app/server/Info.plist.in" > "$og_app_dest/Contents/Info.plist"
plutil -lint "$og_app_dest/Contents/Info.plist" >/dev/null
printf 'APPL????' > "$og_app_dest/Contents/PkgInfo"

if [ -f "$og_root/app/resources/OpenGridderServer.icns" ]; then
    cp "$og_root/app/resources/OpenGridderServer.icns" "$og_app_dest/Contents/Resources/OpenGridderServer.icns"
fi

echo "SUCCESS: Built OpenGridder Server.app at $og_app_dest"
