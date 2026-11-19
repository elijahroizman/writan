#!/usr/bin/env bash
# Writes the database of a pacman repository (writan.db) for the packages in a directory, the way
# repo-add would, without needing pacman.
# The release workflow runs it once per architecture and signs the packages and the database.
#
# Each package gets a `<name>-<version>/desc` entry built from its .PKGINFO, with the file name,
# size and SHA-256 sum pacman checks the download against. No PGPSIG field: pacman then fetches
# `<package>.sig` from beside the package. There's no files database (`pacman -F`).
#
# Usage: scripts/pacman-repo.sh <directory with *.pkg.tar.zst>
# Prints the database's path.
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <directory>" >&2
  exit 2
fi
dir="$(cd "$1" && pwd)"
shopt -s nullglob
packages=("$dir"/*.pkg.tar.zst)
if [ "${#packages[@]}" -eq 0 ]; then
  echo "no .pkg.tar.zst in $dir" >&2
  exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir "$work/db"
for package in "${packages[@]}"; do
  pkginfo="$(tar --zstd -xOf "$package" .PKGINFO)"
  field() { printf '%s\n' "$pkginfo" | sed -n "s/^$1 = //p"; }
  name="$(field pkgname)"
  version="$(field pkgver)"
  if [ -z "$name" ] || [ -z "$version" ]; then
    echo "$package has no pkgname or pkgver in .PKGINFO" >&2
    exit 1
  fi
  entry="$work/db/$name-$version"
  if [ -e "$entry" ]; then
    echo "two packages for $name $version in $dir" >&2
    exit 1
  fi
  mkdir "$entry"
  section() {
    local values
    values="$(field "$2")"
    [ -n "$values" ] && printf '%%%s%%\n%s\n\n' "$1" "$values"
    return 0
  }
  {
    printf '%%FILENAME%%\n%s\n\n' "$(basename "$package")"
    section NAME pkgname
    section BASE pkgbase
    section VERSION pkgver
    section DESC pkgdesc
    printf '%%CSIZE%%\n%s\n\n' "$(stat -c %s "$package")"
    section ISIZE size
    printf '%%SHA256SUM%%\n%s\n\n' "$(sha256sum "$package" | cut -d' ' -f1)"
    section URL url
    section LICENSE license
    section ARCH arch
    section BUILDDATE builddate
    section PACKAGER packager
    section REPLACES replaces
    section CONFLICTS conflict
    section PROVIDES provides
    section DEPENDS depend
    section OPTDEPENDS optdepend
    section MAKEDEPENDS makedepend
  } > "$entry/desc"
done

(cd "$work/db" && tar --sort=name --owner=0 --group=0 --numeric-owner -czf "$dir/writan.db" -- *)
echo "$dir/writan.db"
