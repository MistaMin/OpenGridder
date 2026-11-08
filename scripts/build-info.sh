#!/usr/bin/env bash
# Build identity shared by every build script. Source it; values already set in the
# environment are kept so one build stamps the same identity everywhere.
#   OG_VERSION       release version from the VERSION file (e.g. 0.1.1-alpha)
#   OG_BUILD_NUMBER  number of commits on HEAD (rises with every commit)
#   OG_GIT_COMMIT    short commit hash, with "+dirty" when the tree has uncommitted changes
#   OG_BUILD_DATE    local build time, e.g. "2026-11-08 15:42 CET"
_og_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
: "${OG_VERSION:=$(tr -d '[:space:]' < "$_og_root/VERSION")}"
: "${OG_BUILD_NUMBER:=$(git -C "$_og_root" rev-list --count HEAD 2>/dev/null || echo 0)}"
if [ -z "${OG_GIT_COMMIT:-}" ]; then
    OG_GIT_COMMIT="$(git -C "$_og_root" rev-parse --short HEAD 2>/dev/null || echo unknown)"
    if [ -n "$(git -C "$_og_root" status --porcelain --untracked-files=no 2>/dev/null)" ]; then
        OG_GIT_COMMIT="$OG_GIT_COMMIT+dirty"
    fi
fi
: "${OG_BUILD_DATE:=$(date '+%Y-%m-%d %H:%M %Z')}"
export OG_VERSION OG_BUILD_NUMBER OG_GIT_COMMIT OG_BUILD_DATE
unset _og_root
