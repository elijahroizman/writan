#!/usr/bin/env bash
# Checks the Arch package and the pacman repository the release workflow publishes, on a real
# pacman in Arch's own container image (ghcr.io/archlinux/archlinux; needs Docker), with two
# made-up releases of Writan:
#
# - installing the package adds the [writan] repository to pacman.conf and the signing key to
#   pacman's keyring (scripts/arch-package.sh's .INSTALL);
# - pacman reads the database scripts/pacman-repo.sh writes and upgrades Writan from it, checking
#   the signatures on the database and the package;
# - the app's update command, `pacman -U <package URL>`, installs a signed package and refuses a
#   package whose signature doesn't match;
# - removing Writan takes its lines out of pacman.conf and leaves the rest as it was.
#
# The packages and the repository are signed with a throwaway key, which the container also trusts.
# The dependencies are skipped (-dd): they're the real Arch packages, not what's under test.
#
# Then it runs scripts/check-debian-repack.py.
#
# Usage: scripts/check-linux-packages.sh
set -euo pipefail

scripts="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

fake_deb() {
  local version="$1" root="$work/deb-$1"
  mkdir -p "$root/DEBIAN" "$root/usr/bin" "$root/usr/share/applications" "$root/usr/share/keyrings"
  cat > "$root/DEBIAN/control" <<CONTROL
Package: writan
Version: $version
Architecture: amd64
Maintainer: Writan <writan@example.com>
Description: Writan test package
CONTROL
  printf '#!/bin/sh\necho %s\n' "$version" > "$root/usr/bin/writan"
  chmod 755 "$root/usr/bin/writan"
  printf '#!/bin/sh\nexec writan "$@"\n' > "$root/usr/bin/workspace"
  printf '[Desktop Entry]\nName=Writan\nExec=writan\nType=Application\n' > "$root/usr/share/applications/Writan.desktop"
  : > "$root/usr/share/keyrings/writan-archive-keyring.gpg"
  dpkg-deb --root-owner-group -b "$root" "$work/Writan_${version}_amd64.deb" > /dev/null
}
fake_deb 0.10.11
fake_deb 0.10.12

old="$("$scripts/arch-package.sh" "$work/Writan_0.10.11_amd64.deb" "$work/old")"
repo="$work/repo"
new="$("$scripts/arch-package.sh" "$work/Writan_0.10.12_amd64.deb" "$repo")"
tar --zstd -tf "$new" | grep -qx 'usr/share/pacman/keyrings/writan.gpg'
if tar --zstd -tf "$new" | grep -Eq '^usr/(bin/workspace|share/keyrings/)'; then
  echo "the Arch package kept the APT keyring or the workspace wrapper" >&2
  exit 1
fi

export GNUPGHOME="$work/gnupg"
mkdir -m 700 "$GNUPGHOME"
gpg --batch --quiet --passphrase '' --quick-gen-key 'Writan test <test@example.com>' rsa2048 sign never
gpg --batch --armor --export > "$work/test-key.asc"
fingerprint="$(gpg --batch --with-colons --list-keys | awk -F: '$1 == "fpr" { print $10; exit }')"
gpg --batch --detach-sign -o "$new.sig" "$new"
db="$("$scripts/pacman-repo.sh" "$repo")"
gpg --batch --detach-sign -o "$db.sig" "$db"
# A copy of the new package with a signature that belongs to another file.
mkdir "$work/bad"
cp "$new" "$work/bad/"
gpg --batch --detach-sign -o "$work/bad/$(basename "$new").sig" "$old"
gpgconf --kill gpg-agent 2> /dev/null || true

chmod -R a+rX "$work"
docker run --rm \
  -v "$work:/work:ro" \
  -e OLD="/work/old/$(basename "$old")" \
  -e NEW="$(basename "$new")" \
  -e FINGERPRINT="$fingerprint" \
  ghcr.io/archlinux/archlinux:latest bash -euo pipefail -c '
    pacman-key --init > /dev/null 2>&1
    # The container has no sync databases for core and extra; their warnings are noise here.
    pacman() { command pacman "$@" 2> >(grep -v "^warning: database file for" >&2); }
    # Containers have no Landlock, which pacman 7 sandboxes downloads with.
    grep -q "^DisableSandbox" /etc/pacman.conf || sed -i "/^\[options\]/a DisableSandbox" /etc/pacman.conf
    cp /etc/pacman.conf /tmp/pacman.conf.before

    pacman -U --noconfirm -dd "$OLD" > /dev/null
    grep -qx "\[writan\]" /etc/pacman.conf
    grep -qx "SigLevel = Required" /etc/pacman.conf
    grep -qx "Server = https://github.com/elijahroizman/writan/releases/download/arch/\$arch" /etc/pacman.conf
    [ "$(grep -c "^\[writan\]" /etc/pacman.conf)" = 1 ]
    pacman-key --list-keys B7CEFA4542D27FD8F333C19631DA0E6C8EFB89F5 > /dev/null
    pacman -Qqo /usr/bin/writan | grep -qx writan
    echo "install adds the repository and the key"

    pacman-key --add /work/test-key.asc > /dev/null 2>&1
    pacman-key --lsign-key "$FINGERPRINT" > /dev/null 2>&1

    # The app'\''s update command, signature checked, then a bad signature refused.
    if pacman -U --noconfirm -dd "file:///work/bad/$NEW" > /tmp/bad.log 2>&1; then
      echo "pacman installed a package with a wrong signature" >&2
      exit 1
    fi
    [ "$(writan)" = 0.10.11 ]
    # pacman keeps the rejected download in its cache under the same name.
    rm -f /var/cache/pacman/pkg/writan-*
    pacman -U --noconfirm -dd "file:///work/repo/$NEW" > /dev/null
    [ "$(writan)" = 0.10.12 ]
    [ "$(grep -c "^\[writan\]" /etc/pacman.conf)" = 1 ]
    echo "pacman -U installs the signed package and refuses a bad signature"

    pacman -U --noconfirm -dd "$OLD" > /dev/null
    sed -i "s|^Server = https://github.com/elijahroizman/writan/releases/download/arch/\$arch\$|Server = file:///work/repo|" /etc/pacman.conf
    pacman -Sy --noconfirm -dd writan > /dev/null
    [ "$(writan)" = 0.10.12 ]
    pacman -Si writan | grep -q "^Version *: 0.10.12-1"
    echo "pacman upgrades Writan from the signed repository database"

    sed -i "s|^Server = file:///work/repo\$|Server = https://github.com/elijahroizman/writan/releases/download/arch/\$arch|" /etc/pacman.conf
    # What the comment above the section says to do: a # before its three lines.
    sed -i "/^\[writan\]\$/,/^Server = /s|^|#|" /etc/pacman.conf
    pacman -U --noconfirm -dd "$OLD" > /dev/null
    [ "$(grep -c "\[writan\]" /etc/pacman.conf)" = 1 ]
    grep -qx "#\[writan\]" /etc/pacman.conf
    echo "a commented-out repository stays off"

    pacman -R --noconfirm writan > /dev/null
    if [ "$(sha256sum < /tmp/pacman.conf.before)" != "$(sha256sum < /etc/pacman.conf)" ]; then
      tail -n 8 /etc/pacman.conf >&2
      echo "removing Writan left pacman.conf changed" >&2
      exit 1
    fi
    echo "removal restores pacman.conf"
  '

python3 "$scripts/check-debian-repack.py"
