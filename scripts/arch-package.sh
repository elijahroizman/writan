#!/usr/bin/env bash
# Repacks the .deb Tauri builds into an Arch Linux package (pacman -U).
#
# Tauri has no pacman bundle, and the binary is the same one the .deb and .rpm carry, so this
# lifts the payload out of the .deb and writes the metadata pacman reads (.PKGINFO and .INSTALL)
# around it, the way makepkg would. It needs dpkg-deb, GNU tar, zstd and gpg, and runs on the Ubuntu
# release runner; nothing here runs pacman.
#
# Left out of the payload: the APT keyring (the Debian package manager's, and pacman wouldn't use
# it) and /usr/bin/workspace (the wrapper for the pre-rename updater, which never ran on Arch).
# Added: the repository signing key as a pacman keyring (/usr/share/pacman/keyrings/writan*), which
# .INSTALL hands to `pacman-key --populate`, and which signs the packages and the database of
# Writan's pacman repository. .INSTALL also adds the [writan] repository to /etc/pacman.conf, the
# way the .deb's postinst adds the APT one, and removing the package takes it out again.
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
key="$(cd "$(dirname "$0")/.." && pwd)/src-tauri/linux/RPM-GPG-KEY-writan"
fingerprint="$(gpg --batch --show-keys --with-colons "$key" | awk -F: '$1 == "fpr" { print $10; exit }')"
if [ -z "$fingerprint" ]; then
  echo "couldn't read the fingerprint of $key" >&2
  exit 1
fi

version="$(dpkg-deb -f "$deb" Version)"
# pacman versions can't hold a hyphen, and unlike dpkg and rpm, pacman doesn't sort a tilde
# before the release: 1.0.1~nightly.1 is newer than 1.0.1 to pacman. A letter straight after the
# release part is older, so the pre-release's separator (the .deb's tilde, or SemVer's hyphen)
# goes: 1.0.1nightly.202611280930 sorts after 1.0.0 and before 1.0.1, the way SemVer orders
# 1.0.1-nightly.202611280930.
case "$version" in
  *[-~]*)
    pre="${version#*[-~]}"
    case "$pre" in
      [A-Za-z]*) pkgver="${version%%[-~]*}${pre}" ;;
      *)
        echo "can't map the pre-release version $version to pacman: it must start with a letter" >&2
        exit 1
        ;;
    esac
    ;;
  *) pkgver="$version" ;;
esac
case "$pkgver" in
  *[-~:/]*)
    echo "pacman can't use the version $pkgver" >&2
    exit 1
    ;;
esac
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
keyrings="$root/usr/share/pacman/keyrings"
mkdir -p "$keyrings"
cp "$key" "$keyrings/writan.gpg"
printf '%s:4:\n' "$fingerprint" > "$keyrings/writan-trusted"
: > "$keyrings/writan-revoked"
chmod 644 "$keyrings"/writan*

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

# Refresh the caches the desktop reads (the commands may be missing on a minimal install), and add
# or remove Writan's repository. pacman runs this with / as the root.
#
# The repository goes in /etc/pacman.conf, since pacman has no drop-in directory. It is added once:
# when a [writan] line is there already, even commented out, the file is left alone, so commenting
# the section out turns updates off until Writan is removed. Removing Writan deletes the lines it
# added (the marker comment, the blank line before it and the section after it, with a mirror
# the user put in its place), commented out or not, and nothing else.
cat > "$root/.INSTALL" <<'INSTALL'
refresh() {
  command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t -f usr/share/icons/hicolor || true
  command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q usr/share/applications || true
}
add_repo() {
  if ! pacman-key --populate writan >/dev/null 2>&1; then
    echo "Writan: couldn't add its signing key to pacman's keyring. Run: sudo pacman-key --populate writan"
  fi
  [ -f /etc/pacman.conf ] || return 0
  grep -Eq '^[[:space:]]*#*[[:space:]]*\[writan\]' /etc/pacman.conf && return 0
  cat >> /etc/pacman.conf <<'CONF'

# Writan updates (https://github.com/elijahroizman/writan). Put a # before the next three lines to stop them.
[writan]
SigLevel = Required
Server = https://github.com/elijahroizman/writan/releases/download/arch/$arch
CONF
}
remove_repo() {
  [ -f /etc/pacman.conf ] || return 0
  grep -q '^# Writan updates (https://github.com/elijahroizman/writan)' /etc/pacman.conf || return 0
  tmp="$(mktemp)"
  awk '
    skip && /^[[:space:]]*#*[[:space:]]*(\[writan\]|(SigLevel|Server|Include|Usage)[[:space:]]*=)/ { next }
    { skip = 0 }
    /^# Writan updates \(https:\/\/github\.com\/elijahroizman\/writan\)/ { skip = 1; hold = 0; next }
    hold { print held; hold = 0 }
    /^[[:space:]]*$/ { held = $0; hold = 1; next }
    { print }
    END { if (hold) print held }
  ' /etc/pacman.conf > "$tmp" && cat "$tmp" > /etc/pacman.conf
  rm -f "$tmp"
}
post_install() { refresh; add_repo; }
post_upgrade() { refresh; add_repo; }
post_remove() { refresh; remove_repo; }
INSTALL

file="$out/writan-${pkgver}-${pkgrel}-${arch}.pkg.tar.zst"
cd "$root"
# The metadata files go first. .MTREE (which only `pacman -Qkk` reads) is left out.
{
  printf '%s\0' .PKGINFO .INSTALL
  find . -mindepth 1 \( -path ./.PKGINFO -o -path ./.INSTALL \) -prune -o -printf '%P\0' | LC_ALL=C sort -z
} | LC_ALL=C tar --null --no-recursion --files-from=- --owner=0 --group=0 --numeric-owner --mode=go-w -cf - | zstd -19 -T0 -q -f -o "$file"
echo "$file"
