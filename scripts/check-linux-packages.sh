#!/usr/bin/env bash
# Checks the Arch package and the pacman repository the release workflow publishes, on a real
# pacman in Arch's own container image (ghcr.io/archlinux/archlinux; needs Docker), with two
# made-up releases of Writan:
#
# - installing the package adds the [writan] repository to pacman.conf and the signing key to
#   pacman's keyring (scripts/arch-package.sh's .INSTALL);
# - pacman reads the database scripts/pacman-repo.sh writes and upgrades Writan from it, checking
#   the signatures on the database and the package;
# - the app's update command (src-tauri/linux/pacman-install.sh) installs a package signed with
#   Writan's key, and refuses one with no signature, a signature of another file, or a signature
#   from another key pacman trusts, on a pacman.conf that lets pacman install unsigned files;
# - removing Writan takes its lines out of pacman.conf and leaves the rest as it was;
# - a nightly's package (arch-package.sh turns 0.10.13~nightly.<stamp> into 0.10.13nightly.<stamp>)
#   upgrades the stable release before it, and the stable release after it upgrades the nightly.
#
# The packages and the repository are signed with a throwaway key, which the container also trusts.
# The dependencies are skipped (-dd): they're the real Arch packages, not what's under test.
#
# Then it runs scripts/check-debian-repack.py.
#
# Usage: scripts/check-linux-packages.sh
set -euo pipefail

scripts="$(cd "$(dirname "$0")" && pwd)"
install_script="$scripts/../src-tauri/linux/pacman-install.sh"
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
# The nightly's version as scripts/debian-repack.sh leaves it, and the stable release after it.
fake_deb 0.10.13~nightly.202611280930
fake_deb 0.10.13

old="$("$scripts/arch-package.sh" "$work/Writan_0.10.11_amd64.deb" "$work/old")"
repo="$work/repo"
new="$("$scripts/arch-package.sh" "$work/Writan_0.10.12_amd64.deb" "$repo")"
tar --zstd -tf "$new" | grep -qx 'usr/share/pacman/keyrings/writan.gpg'
nightly="$("$scripts/arch-package.sh" "$work/Writan_0.10.13~nightly.202611280930_amd64.deb" "$work/nightly")"
stable="$("$scripts/arch-package.sh" "$work/Writan_0.10.13_amd64.deb" "$work/stable")"
if [ "$(basename "$nightly")" != writan-0.10.13nightly.202611280930-1-x86_64.pkg.tar.zst ]; then
  echo "the nightly's Arch package is $(basename "$nightly")" >&2
  exit 1
fi
if tar --zstd -tf "$new" | grep -Eq '^usr/(bin/workspace|share/keyrings/)'; then
  echo "the Arch package kept the APT keyring or the workspace wrapper" >&2
  exit 1
fi

export GNUPGHOME="$work/gnupg"
mkdir -m 700 "$GNUPGHOME"
gpg --batch --quiet --passphrase '' --quick-gen-key 'Writan test <test@example.com>' rsa2048 sign never
gpg --batch --armor --export test@example.com > "$work/test-key.asc"
fingerprint="$(gpg --batch --with-colons --list-keys | awk -F: '$1 == "fpr" { print $10; exit }')"
sign() { gpg --batch --local-user test@example.com --detach-sign "$@"; }
sign -o "$new.sig" "$new"
db="$("$scripts/pacman-repo.sh" "$repo")"
sign -o "$db.sig" "$db"
# Copies of the new package: with no signature, with a signature that belongs to another file,
# and signed by a key pacman trusts that isn't Writan's.
mkdir "$work/unsigned" "$work/bad" "$work/stranger"
cp "$new" "$work/unsigned/"
cp "$new" "$work/bad/"
sign -o "$work/bad/$(basename "$new").sig" "$old"
gpg --batch --quiet --passphrase '' --quick-gen-key 'Someone else <other@example.com>' rsa2048 sign never
gpg --batch --armor --export other@example.com > "$work/other-key.asc"
other_fingerprint="$(gpg --batch --with-colons --list-keys other@example.com | awk -F: '$1 == "fpr" { print $10; exit }')"
cp "$new" "$work/stranger/"
gpg --batch --local-user other@example.com --detach-sign -o "$work/stranger/$(basename "$new").sig" "$new"
gpgconf --kill gpg-agent 2> /dev/null || true
cp "$install_script" "$work/pacman-install.sh"

chmod -R a+rX "$work"
docker run --rm \
  -v "$work:/work:ro" \
  -e OLD="/work/old/$(basename "$old")" \
  -e NEW="$(basename "$new")" \
  -e FINGERPRINT="$fingerprint" \
  -e OTHER_FINGERPRINT="$other_fingerprint" \
  -e NIGHTLY="/work/nightly/$(basename "$nightly")" \
  -e STABLE="/work/stable/$(basename "$stable")" \
  ghcr.io/archlinux/archlinux:latest bash -euo pipefail -c '
    pacman-key --init > /dev/null 2>&1
    # The container has no sync databases for core and extra; their warnings are noise here.
    pacman() { command pacman "$@" 2> >(grep -v "^warning: database file for" >&2); }
    # Containers have no Landlock, which pacman 7 sandboxes downloads with.
    grep -q "^DisableSandbox" /etc/pacman.conf || sed -i "/^\[options\]/a DisableSandbox" /etc/pacman.conf
    # pacman alone would install an unsigned file at these levels; the update command must not.
    sed -i "/^[#[:space:]]*\(Local\|Remote\)FileSigLevel/d" /etc/pacman.conf
    sed -i "/^\[options\]/a LocalFileSigLevel = Optional\nRemoteFileSigLevel = Optional" /etc/pacman.conf
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
    pacman-key --add /work/other-key.asc > /dev/null 2>&1
    pacman-key --lsign-key "$OTHER_FINGERPRINT" > /dev/null 2>&1

    # The app'\''s update command, as pkexec runs it, checked against the test key in place of
    # Writan'\''s. The shim skips the dependencies, like the rest of this test.
    mkdir -p /tmp/shim
    printf "#!/bin/sh\nexec /usr/bin/pacman -dd \"\$@\"\n" > /tmp/shim/pacman
    chmod 755 /tmp/shim/pacman
    update() { PATH="/tmp/shim:$PATH" sh -c "set -- \"\$1\" /work/test-key.asc; $(cat /work/pacman-install.sh)" sh "$1"; }
    for case in unsigned bad stranger; do
      if update "file:///work/$case/$NEW" > "/tmp/$case.log" 2>&1; then
        cat "/tmp/$case.log" >&2
        echo "the update command installed a package with the $case signature" >&2
        exit 1
      fi
      [ "$(writan)" = 0.10.11 ]
    done
    grep -q "Couldn.t download the update or its signature" /tmp/unsigned.log
    grep -q "doesn.t match Writan.s key" /tmp/bad.log
    grep -q "doesn.t match Writan.s key" /tmp/stranger.log
    update "file:///work/repo/$NEW" > /dev/null
    [ "$(writan)" = 0.10.12 ]
    [ "$(grep -c "^\[writan\]" /etc/pacman.conf)" = 1 ]
    echo "the update command installs the signed package and refuses no signature, a bad one and another key'\''s"

    # pacman has no tilde rule: the nightly has to sort before the release it leads up to.
    [ "$(vercmp 0.10.13nightly.202611280930-1 0.10.13-1)" = -1 ]
    [ "$(vercmp 0.10.13nightly.202611280930-1 0.10.12-1)" = 1 ]
    [ "$(vercmp 0.10.13nightly.202611280930-1 0.10.13nightly.202611281200-1)" = -1 ]
    pacman -U --noconfirm -dd "$NIGHTLY" > /tmp/nightly.log 2>&1
    pacman -Q writan | grep -qx "writan 0.10.13nightly.202611280930-1"
    pacman -U --noconfirm -dd "$STABLE" > /tmp/stable.log 2>&1
    if grep -qi downgrad /tmp/nightly.log /tmp/stable.log; then
      cat /tmp/nightly.log /tmp/stable.log >&2
      echo "pacman took a nightly or the release after it as a downgrade" >&2
      exit 1
    fi
    [ "$(writan)" = 0.10.13 ]
    echo "a nightly upgrades the release before it, and the next release upgrades the nightly"

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

    # A mirror of its own in place of the GitHub one goes too.
    sed -i "s|^#Server = https://github.com/elijahroizman/writan/releases/download/arch/\$arch\$|#Server = https://mirror.example.com/writan/\$arch|" /etc/pacman.conf
    grep -qx "#Server = https://mirror.example.com/writan/\$arch" /etc/pacman.conf
    pacman -R --noconfirm writan > /dev/null
    if [ "$(sha256sum < /tmp/pacman.conf.before)" != "$(sha256sum < /etc/pacman.conf)" ]; then
      tail -n 8 /etc/pacman.conf >&2
      echo "removing Writan left pacman.conf changed" >&2
      exit 1
    fi
    echo "removal restores pacman.conf"
  '

python3 "$scripts/check-debian-repack.py"
