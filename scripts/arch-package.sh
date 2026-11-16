#!/usr/bin/env bash
# Repacks the .deb Tauri builds into an Arch Linux package (pacman -U).
#
# Tauri has no pacman bundle, and the binary is the same one the .deb and .rpm carry, so this
# lifts the payload out of the .deb and writes the metadata pacman reads (.PKGINFO and .INSTALL)
# around it, the way makepkg would. It needs dpkg-deb, GNU tar and zstd, and runs on the Ubuntu
# release runner; nothing here runs pacman.
#
# Left out of the payload: the APT keyring (the Debian package manager's, and pacman wouldn't use
# it) and /usr/bin/workspace (the wrapper for the pre-rename updater, which never ran on Arch).
#
# The package's architecture follows the .deb's: amd64 becomes x86_64 (Arch Linux) and arm64
# becomes aarch64 (Arch Linux ARM, and Omarchy on ARM machines).
#
# Usage: scripts/arch-package.sh <Writan_x.y.z_amd64.deb | Writan_x.y.z_arm64.deb> <output directory>
# Prints the package's path.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <deb> <output directory>" >&2
  exit 2
fi
deb="$1"
out="$2"

version="$(dpkg-deb -f "$deb" Version)"
# pacman versions can't hold a hyphen. SemVer's pre-release separator becomes a tilde, which
# pacman sorts before the release the way SemVer does.
pkgver="${version//-/~}"
pkgrel=1
case "$(dpkg-deb -f "$deb" Architecture)" in
  amd64) arch=x86_64 ;;
  arm64) arch=aarch64 ;;
  *)
    echo "no Arch Linux architecture for the .deb's $(dpkg-deb -f "$deb" Architecture)" >&2
    exit 1
    ;;
esac
description="$(dpkg-deb -f "$deb" Description | head -n 1)"
created="${SOURCE_DATE_EPOCH:-$(date +%s)}"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
root="$work/root"
mkdir -p "$root" "$out"
dpkg-deb --extract "$deb" "$root"
rm -f "$root/usr/bin/workspace"
rm -rf "$root/usr/share/keyrings"
rmdir "$root/usr/share" 2>/dev/null || true

if [ ! -x "$root/usr/bin/writan" ]; then
  echo "the .deb has no /usr/bin/writan" >&2
  exit 1
fi

size="$(du -sb "$root" | cut -f1)"
cat > "$root/.PKGINFO" <<PKGINFO
pkgname = writan
pkgbase = writan
pkgver = ${pkgver}-${pkgrel}
pkgdesc = ${description}
url = https://github.com/elijahroizman/writan
builddate = ${created}
packager = Unknown Packager
size = ${size}
arch = ${arch}
license = custom
depend = gtk3
depend = webkit2gtk-4.1
depend = libayatana-appindicator
depend = openssl
depend = xdotool
optdepend = hunspell-en_us: spell checking for English (any hunspell-* dictionary works)
PKGINFO

# Refresh the caches the desktop reads. The commands may be missing on a minimal install.
cat > "$root/.INSTALL" <<'INSTALL'
refresh() {
  command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t -f usr/share/icons/hicolor || true
  command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q usr/share/applications || true
}
post_install() { refresh; }
post_upgrade() { refresh; }
post_remove() { refresh; }
INSTALL

file="$out/writan-${pkgver}-${pkgrel}-${arch}.pkg.tar.zst"
cd "$root"
# The metadata files go first. .MTREE (which only `pacman -Qkk` reads) is left out.
{
  printf '%s\0' .PKGINFO .INSTALL
  find . -mindepth 1 \( -path ./.PKGINFO -o -path ./.INSTALL \) -prune -o -printf '%P\0' | LC_ALL=C sort -z
} | LC_ALL=C tar --null --no-recursion --files-from=- --owner=0 --group=0 --numeric-owner --mode=go-w -cf - | zstd -19 -T0 -q -f -o "$file"
echo "$file"
