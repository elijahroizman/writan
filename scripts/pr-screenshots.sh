#!/usr/bin/env bash
# Upload screenshots for a pull request and print the Markdown to paste into its description.
#
#   scripts/pr-screenshots.sh before.png after.png
#
# The pictures go on the `pr-screenshots` branch, in a folder named after the current branch,
# so they never land on main. The branch is made the first time this runs.
set -euo pipefail

if [ "$#" -eq 0 ]; then
  echo "usage: scripts/pr-screenshots.sh picture.png [more.png ...]" >&2
  exit 1
fi
for f in "$@"; do
  [ -f "$f" ] || { echo "not a file: $f" >&2; exit 1; }
done

store=pr-screenshots
branch="$(git rev-parse --abbrev-ref HEAD)"
folder="${branch//\//-}"
repo="$(gh repo view --json nameWithOwner -q .nameWithOwner)"
index="$(mktemp -u)"
trap 'rm -f "$index"' EXIT
export GIT_INDEX_FILE="$index"

name_of() { basename "$1" | tr ' ' '-'; }

for attempt in 1 2 3; do
  rm -f "$index"
  parent=""
  if git fetch -q origin "$store" 2>/dev/null; then
    parent="$(git rev-parse FETCH_HEAD)"
    git read-tree "$parent"
  else
    git read-tree --empty
  fi
  for f in "$@"; do
    blob="$(git hash-object -w "$f")"
    git update-index --add --cacheinfo "100644,$blob,$folder/$(name_of "$f")"
  done
  tree="$(git write-tree)"
  commit="$(git commit-tree "$tree" ${parent:+-p "$parent"} -m "Screenshots for $branch")"
  if git push -q origin "$commit:refs/heads/$store"; then
    for f in "$@"; do
      n="$(name_of "$f")"
      echo "![${n%.*}](https://github.com/$repo/blob/$store/$folder/$n?raw=true)"
    done
    exit 0
  fi
  echo "push failed (attempt $attempt), retrying" >&2
  sleep 2
done
exit 1
