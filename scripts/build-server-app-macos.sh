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

# Bundle the Homebrew LV2 libraries (lilv and what it links) so the app runs on a Mac
# without Homebrew. Every reference is rewritten to Contents/Frameworks.
og_frameworks="$og_app_dest/Contents/Frameworks"
mkdir -p "$og_frameworks"
# Newline-separated list (macOS bash 3.2 with set -u rejects empty arrays).
og_bundle_libs=""
og_collect() {
    local dep
    for dep in $(otool -L "$1" | tail -n +2 | awk '{print $1}' | grep '^/opt/homebrew/' || true); do
        if ! printf '%s\n' "$og_bundle_libs" | grep -qxF "$dep"; then
            og_bundle_libs="$og_bundle_libs$dep
"
            og_collect "$dep"
        fi
    done
}
og_collect "$og_app_dest/Contents/MacOS/OpenGridderServer-bin"
for dep in $og_bundle_libs; do
    name="$(basename "$dep")"
    cp -L "$dep" "$og_frameworks/$name"
    chmod u+w "$og_frameworks/$name"
    install_name_tool -id "@executable_path/../Frameworks/$name" "$og_frameworks/$name" 2>/dev/null
done
for target in "$og_app_dest/Contents/MacOS/OpenGridderServer-bin" "$og_frameworks"/*.dylib; do
    [ -e "$target" ] || continue
    for dep in $og_bundle_libs; do
        install_name_tool -change "$dep" "@executable_path/../Frameworks/$(basename "$dep")" "$target" 2>/dev/null
    done
done
if otool -L "$og_app_dest/Contents/MacOS/OpenGridderServer-bin" | grep -q '/opt/homebrew/' ||
   { ls "$og_frameworks"/*.dylib >/dev/null 2>&1 && otool -L "$og_frameworks"/*.dylib | grep -q '/opt/homebrew/'; }; then
    echo "ERROR: a Homebrew library is still referenced"; exit 1
fi
echo "Bundled $(printf '%s' "$og_bundle_libs" | grep -c .) libraries into Contents/Frameworks"

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
