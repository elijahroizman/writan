# Installs one of Writan's Arch packages after checking its signature against Writan's own key.
#
# The Update button runs this as root through pkexec (`package_upgrade` in src-tauri/src/update.rs
# puts `set -- <package URL> <keyring>` in front of it), and scripts/check-linux-packages.sh runs it
# on a real pacman.
#
# `pacman -U <URL>` would check the signature at pacman.conf's RemoteFileSigLevel, which a system
# can set to Optional, and against any key pacman trusts. So this downloads the package and its
# detached signature (`<URL>.sig`) into a directory only root can write, checks them with gpgv
# against the keyring the Arch package installed and nothing else, and then installs that file. A
# missing signature fails the same way as a wrong one. pacman still checks the signature beside the
# file against its own keyring, at LocalFileSigLevel.
set -eu
url="$1"
keyring="$2"
name="${url##*/}"
d="$(mktemp -d)"
trap 'rm -rf "$d"' EXIT
mkdir -m 700 "$d/gnupg"
if ! curl -fsSL --proto-redir '=https' -o "$d/$name" "$url" || ! curl -fsSL --proto-redir '=https' -o "$d/$name.sig" "$url.sig"; then
  echo "Couldn't download the update or its signature. Check your connection and try again." >&2
  exit 1
fi
# The keyring is an armored key (a copy of RPM-GPG-KEY-writan), which gpgv can't read.
if ! gpg --homedir "$d/gnupg" --batch --quiet --dearmor < "$keyring" > "$d/writan.keyring"; then
  echo "Couldn't read Writan's signing key in $keyring. Reinstall Writan." >&2
  exit 1
fi
if ! gpgv --homedir "$d/gnupg" --keyring "$d/writan.keyring" "$d/$name.sig" "$d/$name" 2> "$d/gpgv.log"; then
  echo "The update's signature doesn't match Writan's key. It wasn't installed." >&2
  exit 1
fi
pacman -U --noconfirm "$d/$name"
