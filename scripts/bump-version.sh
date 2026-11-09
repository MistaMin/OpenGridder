#!/usr/bin/env bash
# Bump the OpenGridder product version in the version file, workspace and source strings.
#   scripts/bump-version.sh 0.1.2-alpha
set -euo pipefail
new="${1:?usage: bump-version.sh <version>}"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
old="$(tr -d '[:space:]' < "$root/VERSION")"
test -n "$old"
test "$new" != "$old"
echo "Bumping OpenGridder $old -> $new"
printf '%s\n' "$new" > "$root/VERSION"
files="$(git -C "$root" grep -l -F "$old" -- Cargo.toml crates scripts README.md LEGACY.md 2>/dev/null || true)"
for file in $files; do
    sed -i '' "s|$old|$new|g" "$root/$file"
done
echo "Updated:"
printf '%s\n' "$files"
echo "Run cargo build/test once so Cargo.lock records the workspace version."
