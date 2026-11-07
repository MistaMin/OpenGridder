#!/usr/bin/env bash
set -euo pipefail

NEW_VER="0.1.1-alpha"
echo "Bumping version to $NEW_VER..."

echo "$NEW_VER" > /Volumes/Developer/OpenGrid/audiogridder-open/VERSION

sed -i '' 's/version = "0.1.0-alpha"/version = "0.1.1-alpha"/g' /Volumes/Developer/OpenGrid/audiogridder-open/Cargo.toml
sed -i '' 's/"0.1.0-alpha"/"0.1.1-alpha"/g' /Volumes/Developer/OpenGrid/audiogridder-open/crates/ag-plugin/src/lib.rs
sed -i '' 's/"0.1.0-alpha"/"0.1.1-alpha"/g' /Volumes/Developer/OpenGrid/audiogridder-open/crates/ag-server/src/lib.rs
sed -i '' 's/"0.1.0-alpha"/"0.1.1-alpha"/g' /Volumes/Developer/OpenGrid/audiogridder-open/crates/ag-server/src/main.rs
sed -i '' 's/"0.1.0-alpha"/"0.1.1-alpha"/g' /Volumes/Developer/OpenGrid/audiogridder-open/crates/ag-server/src/menu.rs
sed -i '' 's/"0.1.0-alpha"/"0.1.1-alpha"/g' /Volumes/Developer/OpenGrid/audiogridder-open/crates/ag-server/src/mdns.rs
sed -i '' 's/"0.1.0-alpha"/"0.1.1-alpha"/g' /Volumes/Developer/OpenGrid/audiogridder-open/crates/ag-tray/src/main.rs
sed -i '' 's/"0.1.0-alpha"/"0.1.1-alpha"/g' /Volumes/Developer/OpenGrid/audiogridder-open/crates/ag-server/native/host.cpp
sed -i '' 's/0.1.0-alpha/0.1.1-alpha/g' /Volumes/Developer/OpenGrid/audiogridder-open/.github/workflows/crossplatform-legacy.yml

echo "Updated files. Updating Cargo.lock..."
cd /Volumes/Developer/OpenGrid/audiogridder-open
cargo check --workspace
echo "Version bumped to $NEW_VER successfully."
