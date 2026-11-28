#!/usr/bin/env bash
# Writes latest.json, the feed installed Mac and Windows copies check, from the update bundles
# and signatures on a draft release. The Release workflow publishes it as the stable feed
# (plugins.updater.endpoints in src-tauri/tauri.conf.json), the Nightly workflow as the nightly
# feed (NIGHTLY_FEED in src-tauri/src/update.rs). tauri-action writes it like this too, with the
# release body as the notes. It lists the Apple silicon bundle under darwin-aarch64 and the Intel
# one under darwin-x86_64, so the universal app (0.11 to 0.15) updates to the build for the Mac it
# runs on.
#
# Fails, leaving the release a draft, if the release isn't a draft or a bundle or its signature
# is missing.
#
# <name> is the version as the Windows installer's file name has it, when that isn't <version>: a
# nightly 1.1.0-nightly.202611280930 has Writan_1.1.0-nightly-2026-11-28-0930_x64-setup.exe
# (scripts/nightly.mjs names).
#
# Usage: GH_TOKEN=... GITHUB_REPOSITORY=owner/repo scripts/update-feed.sh <tag> <version> <out-dir> [<name>]
set -euo pipefail

if [ "$#" -ne 3 ] && [ "$#" -ne 4 ]; then
  echo "usage: $0 <tag> <version> <out-dir> [<name>]" >&2
  exit 2
fi
tag="$1"
version="$2"
out="$(cd "$3" && pwd)"
name="${4:-$version}"
base="https://github.com/${GITHUB_REPOSITORY}/releases/download/${tag}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cd "$work"
gh release view "$tag" -R "$GITHUB_REPOSITORY" --json body,assets,isDraft > release.json
if ! jq -e '.isDraft' release.json > /dev/null; then
  echo "::error::$tag is already public; refusing to replace live update assets."
  exit 1
fi
gh release download "$tag" -R "$GITHUB_REPOSITORY" -p '*.sig'
platforms='{}'
# Each update bundle under the keys Tauri's updater looks up: {os}-{arch}-{installer}, then {os}-{arch}.
add() {
  local file="$1"
  shift
  if [ ! -s "$file.sig" ] || ! jq -e --arg f "$file" 'any(.assets[]; .name == $f and .size > 0)' release.json > /dev/null; then
    echo "::error::Missing $file or its signature; leaving $tag in draft."
    exit 1
  fi
  for key in "$@"; do
    platforms="$(jq --arg key "$key" --arg url "$base/$file" --rawfile sig "$file.sig" \
      '.[$key] = {signature: $sig, url: $url}' <<< "$platforms")"
  done
}
add "Writan_${name}_x64-setup.exe" windows-x86_64 windows-x86_64-nsis
add Writan_aarch64.app.tar.gz darwin-aarch64 darwin-aarch64-app
add Writan_x64.app.tar.gz darwin-x86_64 darwin-x86_64-app
jq -n \
  --arg version "$version" \
  --arg pub_date "$(date -u +%Y-%m-%dT%H:%M:%S.000Z)" \
  --slurpfile release release.json \
  --argjson platforms "$platforms" \
  '{version: $version, notes: $release[0].body, pub_date: $pub_date, platforms: $platforms}' > "$out/latest.json"
jq '.platforms | map_values(.url)' "$out/latest.json"
