#!/usr/bin/env bash
# Rewrites a .deb Tauri built for the APT repository, in place.
#
# Tauri copies the app's SemVer version into the control file as it is. dpkg reads everything
# after the last hyphen as the Debian revision, so a nightly's 1.1.0-nightly.202611280930 would be
# version 1.1.0 revision nightly.202611280930, which sorts after 1.1.0 and turns the next stable
# release into a downgrade. A pre-release's hyphen becomes a tilde, 1.1.0~nightly.202611280930,
# which dpkg sorts before 1.1.0. Stable versions have no hyphen and stay as they are.
#
# Tauri writes Provides and Replaces but has no Breaks field, so this adds
# `Breaks: workspace (<< <version>)`: the package renamed from workspace to writan in 0.10.0, and the
# transitional workspace package at the same version still installs beside it. The rebuilt package's
# files stay root-owned (extraction runs as the Actions user), and the script fails if any isn't.
#
# Usage: scripts/debian-repack.sh <Writan_x.y.z_amd64.deb | Writan_x.y.z_arm64.deb>
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <deb>" >&2
  exit 2
fi
deb_path="$1"
version="$(dpkg-deb -f "$deb_path" Version)"
case "$version" in
  *-*) version="${version/-/\~}" ;;
esac
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
dpkg-deb -R "$deb_path" "$work/pkg"
sed -i "s/^Version: .*/Version: ${version}/" "$work/pkg/DEBIAN/control"
if ! grep -q '^Replaces:' "$work/pkg/DEBIAN/control"; then
  echo "deb control has no Replaces field. Set bundle.linux.deb.replaces in tauri.linux.conf.json." >&2
  exit 1
fi
if grep -q '^Breaks:' "$work/pkg/DEBIAN/control"; then
  echo "deb control already has a Breaks field" >&2
  exit 1
fi
sed -i "/^Replaces:/a Breaks: workspace (<< ${version})" "$work/pkg/DEBIAN/control"
dpkg-deb --root-owner-group -b "$work/pkg" "$deb_path" > /dev/null
dpkg-deb --fsys-tarfile "$deb_path" | tar --numeric-owner -tvf - | \
  awk '{ seen = 1; if ($2 != "0/0") { print "Non-root package owner: " $0; bad = 1 } } END { exit (!seen || bad) }'
