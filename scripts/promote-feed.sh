#!/usr/bin/env bash
# Points the nightly update feed (latest.json on the pre-release tagged "nightly", NIGHTLY_FEED in
# src-tauri/src/update.rs) at a published release, when that release is newer than the version
# the feed has. The Nightly workflow runs it for each nightly and the Release workflow for each
# stable release, so copies on the nightly channel move onto a stable release newer than their
# nightly.
#
# It copies the release's own latest.json asset, which scripts/update-feed.sh wrote before the
# release was published, so it works the same on a rerun: a version the feed already has is left
# alone. Both workflows run it in one concurrency group (writan-update-feed), so one run's read,
# comparison and upload can't interleave with another's: a nightly that finishes building after a
# stable release can't put its older version back over it.
#
# A read that fails for any reason other than GitHub saying the release isn't there (HTTP 404) is
# retried, then fails the job, rather than being taken for an empty feed.
#
# The tag is a stable release's (v1.0.0) or a nightly's (v1.1.0-nightly-2026-11-28-0930, whose
# latest.json has the version 1.1.0-nightly.202611280930; scripts/nightly.mjs tag-version).
#
# Usage: GH_TOKEN=... GITHUB_REPOSITORY=owner/repo scripts/promote-feed.sh <tag>
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <tag>" >&2
  exit 2
fi
tag="$1"
repo="$GITHUB_REPOSITORY"
scripts="$(cd "$(dirname "$0")" && pwd)"
version="$(node "$scripts/nightly.mjs" tag-version "$tag")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
# Seconds between attempts; the tests set it to 0.
pause="${PROMOTE_FEED_PAUSE:-10}"

# gh api <args>, written to the file $1. Returns 2 when GitHub answers 404 and 0 on success, and
# retries anything else a few times before failing.
api() {
  local out="$1" attempt
  shift
  for attempt in 1 2 3 4; do
    if gh api "$@" > "$out" 2> "$work/error"; then
      return 0
    fi
    if grep -q 'HTTP 404' "$work/error"; then
      return 2
    fi
    cat "$work/error" >&2
    [ "$attempt" = 4 ] || sleep "$((attempt * pause))"
  done
  echo "::error::Couldn't read gh api $*" >&2
  return 1
}

# The release, which has to be public: GitHub doesn't find a draft by its tag, and the feed's
# URLs only download once it's public.
status=0
api "$work/release.json" "repos/$repo/releases/tags/$tag" || status=$?
if [ "$status" = 2 ]; then
  echo "::error::$tag isn't published, so the nightly feed can't point at it." >&2
  exit 1
elif [ "$status" != 0 ]; then
  exit 1
fi
asset="$(jq -r '[.assets[] | select(.name == "latest.json" and .state == "uploaded")][0].id // empty' "$work/release.json")"
if [ -z "$asset" ]; then
  echo "::error::$tag has no latest.json." >&2
  exit 1
fi
# An asset the release lists has to download: a 404 here fails like any other error.
asset() {
  api "$1" -H 'Accept: application/octet-stream' "repos/$repo/releases/assets/$2" && return 0
  echo "::error::Couldn't download $3." >&2
  return 1
}
asset "$work/latest.json" "$asset" "$tag's latest.json"
if [ "$(jq -r .version "$work/latest.json")" != "$version" ]; then
  echo "::error::$tag's latest.json isn't for $version." >&2
  exit 1
fi

# The feed. A missing "nightly" pre-release is made here; a missing latest.json on it is an empty
# feed. Anything else that can't be read stops here.
status=0
api "$work/nightly.json" "repos/$repo/releases/tags/nightly" || status=$?
if [ "$status" = 2 ]; then
  gh release create nightly -R "$repo" --target "${GITHUB_SHA:-main}" --prerelease --latest=false --title "Nightly update feed" \
    --notes "The update feed copies on the nightly channel check. Nothing here needs downloading by hand."
  api "$work/nightly.json" "repos/$repo/releases/tags/nightly" || exit 1
elif [ "$status" != 0 ]; then
  exit 1
fi
# An asset whose upload never finished isn't served, so it counts as no feed.
current_asset="$(jq -r '[.assets[] | select(.name == "latest.json" and .state == "uploaded")][0].id // empty' "$work/nightly.json")"
if [ -n "$current_asset" ]; then
  asset "$work/current.json" "$current_asset" "the nightly feed's latest.json"
  current="$(jq -er .version "$work/current.json")"
  if ! node "$scripts/nightly.mjs" newer "$version" "$current"; then
    echo "The nightly feed has $current, which isn't older than $version; leaving it."
    exit 0
  fi
  echo "The nightly feed has $current; pointing it at $version."
else
  echo "The nightly feed is empty; pointing it at $version."
fi

for attempt in 1 2 3; do
  if gh release upload nightly -R "$repo" "$work/latest.json" --clobber; then
    exit 0
  fi
  [ "$attempt" = 3 ] || sleep "$((attempt * pause))"
done
echo "::error::Couldn't upload the nightly feed." >&2
exit 1
