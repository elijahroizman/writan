#!/usr/bin/env bash
# Upload screenshots for a pull request and print the Markdown to paste into its description.
#
#   scripts/pr-screenshots.sh before.png after.png
#
# The pictures go on the `pr-screenshots` branch of the remote, never on main. The branch is made
# the first time this runs. Each run puts its pictures in a folder of their own,
# `<branch>/<UTC time>-<commit>/`, so two runs never overwrite each other's pictures. The printed
# links name the commit on `pr-screenshots` that added them, so they keep showing the same
# pictures after later uploads.
#
# It needs git and permission to push to the remote, `origin` unless PR_SCREENSHOTS_REMOTE names
# another. The remote must be on github.com, since the links point there. Working from a fork,
# set PR_SCREENSHOTS_REMOTE to your fork's remote, or drag the pictures into the pull request's
# description on GitHub, which uploads them for you.
set -euo pipefail

if [ "$#" -eq 0 ]; then
  echo "usage: scripts/pr-screenshots.sh picture.png [more.png ...]" >&2
  exit 1
fi
for f in "$@"; do
  [ -f "$f" ] || { echo "not a file: $f" >&2; exit 1; }
done

store=pr-screenshots
remote="${PR_SCREENSHOTS_REMOTE:-origin}"

# owner/repo from the address this pushes to, as it's written in the config.
url="$(git config --get "remote.$remote.pushurl" || git config --get "remote.$remote.url" || true)"
[ -n "$url" ] || { echo "no remote named $remote" >&2; exit 1; }
repo="$(printf '%s\n' "$url" | sed -nE 's#^(https://([^@/]+@)?github\.com/|ssh://([^@/]+@)?github\.com/|([^@/]+@)?github\.com:)([^/]+/[^/]+)/?$#\5#p')"
repo="${repo%.git}"
case "$repo" in
  */*) ;;
  *) echo "$remote is $url, which isn't a GitHub repository" >&2; exit 1 ;;
esac

branch="$(git symbolic-ref -q --short HEAD || echo detached)"
stamp="$(date -u +%Y%m%d-%H%M%S)-$(git rev-parse --short HEAD)"

# Percent-encodes one path segment, byte by byte.
encode() {
  local LC_ALL=C s="$1" out="" c i
  for ((i = 0; i < ${#s}; i++)); do
    c="${s:i:1}"
    case "$c" in
      [A-Za-z0-9._~-]) out+="$c" ;;
      *) out+="$(printf '%%%02X' "'$c")" ;;
    esac
  done
  printf '%s' "$out"
}

# Each part of a path, encoded, joined with `/`.
encode_path() {
  local part out="" parts=()
  IFS=/ read -r -a parts <<<"$1"
  for part in "${parts[@]}"; do out+="${out:+/}$(encode "$part")"; done
  printf '%s' "$out"
}

# The pictures go into the repository's objects first, while their paths still work, then
# everything runs from the top, where index paths start.
pictures=("$@")
blobs=()
for f in "${pictures[@]}"; do blobs+=("$(git hash-object -w -- "$f")"); done
cd "$(git rev-parse --show-toplevel)"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
export GIT_INDEX_FILE="$work/index"

for attempt in 1 2 3; do
  rm -f "$GIT_INDEX_FILE"
  parent=""
  if git fetch -q "$remote" "refs/heads/$store" 2>/dev/null; then
    parent="$(git rev-parse FETCH_HEAD)"
    git read-tree "$parent"
  else
    git read-tree --empty
  fi
  # A run in the same second on the same commit gets its own folder too.
  folder="$branch/$stamp"
  n=2
  while [ -n "$parent" ] && git cat-file -e "$parent:$folder" 2>/dev/null; do
    folder="$branch/$stamp-$n"
    n=$((n + 1))
  done
  # macOS's bash 3.2 has no associative arrays, so the names used so far are a list.
  paths=()
  taken=$'\n'
  for i in "${!pictures[@]}"; do
    name="$(basename "${pictures[$i]}")"
    path="$folder/$name"
    # Two pictures with the same name, from different folders, both stay.
    n=2
    while case "$taken" in *$'\n'"$path"$'\n'*) true ;; *) false ;; esac; do
      case "$name" in
        ?*.*) path="$folder/${name%.*}-$n.${name##*.}" ;;
        *) path="$folder/$name-$n" ;;
      esac
      n=$((n + 1))
    done
    taken+="$path"$'\n'
    git update-index --add --cacheinfo 100644 "${blobs[$i]}" "$path"
    paths+=("$path")
  done
  tree="$(git write-tree)"
  commit="$(git commit-tree "$tree" ${parent:+-p "$parent"} -m "Screenshots for $branch")"
  if git push -q "$remote" "$commit:refs/heads/$store"; then
    for path in "${paths[@]}"; do
      alt="$(basename "$path")"
      alt="${alt%.*}"
      alt="${alt//[\[\]]/}"
      echo "![$alt](https://github.com/$repo/blob/$commit/$(encode_path "$path")?raw=true)"
    done
    exit 0
  fi
  echo "push failed (attempt $attempt), retrying" >&2
  sleep 2
done
exit 1
