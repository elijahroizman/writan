#!/usr/bin/env python3
"""Exercise scripts/debian-repack.sh, which the release workflow runs on each .deb, without installing a package."""
import io
from pathlib import Path
import subprocess
import tarfile
import tempfile

root = Path(__file__).resolve().parents[1]
def repack(folder, version):
    base = Path(folder)
    tree = base / 'package'
    (tree / 'DEBIAN').mkdir(parents=True)
    (tree / 'DEBIAN/control').write_text(f'Package: writan\nVersion: {version}\nArchitecture: amd64\nMaintainer: Test <test@example.com>\nReplaces: workspace\nDescription: Repack regression fixture\n')
    for name in ['usr/bin/writan', 'usr/share/keyrings/writan-archive-keyring.gpg']:
        path = tree / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(b'fixture\n')
    deb = base / f'Writan_{version}_amd64.deb'
    subprocess.run(['dpkg-deb', '--root-owner-group', '--build', str(tree), str(deb)], check=True, capture_output=True)
    subprocess.run([str(root / 'scripts/debian-repack.sh'), str(deb)], check=True)
    data = subprocess.check_output(['dpkg-deb', '--fsys-tarfile', str(deb)])
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        entries = archive.getmembers()
        assert entries and all(m.uid == 0 and m.gid == 0 for m in entries)
        assert archive.getmember('./usr/bin/writan').uid == 0
        assert archive.getmember('./usr/share/keyrings/writan-archive-keyring.gpg').uid == 0
    field = lambda name: subprocess.check_output(['dpkg-deb', '-f', str(deb), name], text=True).strip()
    return field('Version'), field('Breaks')


def older(a, b):
    return subprocess.run(['dpkg', '--compare-versions', a, 'lt', b]).returncode == 0


with tempfile.TemporaryDirectory(prefix='writan-deb-repack-') as folder:
    assert repack(folder, '0.10.5') == ('0.10.5', 'workspace (<< 0.10.5)')

# Tauri writes a nightly's SemVer version as it is. The repack turns the hyphen into a tilde, so
# dpkg sorts the nightly, a pre-release of the next minor version, after the stable release
# before it (and a hotfix of it) and before the next minor one.
with tempfile.TemporaryDirectory(prefix='writan-deb-repack-') as folder:
    nightly = '1.1.0~nightly.202611280930'
    assert repack(folder, '1.1.0-nightly.202611280930') == (nightly, f'workspace (<< {nightly})')
    assert older('1.0.0', nightly) and older('1.0.1', nightly) and older(nightly, '1.1.0')
    assert older(nightly, '1.1.0~nightly.202611281200')
    assert older('1.1.0', '1.2.0~nightly.202612010000')
    # What dpkg would make of the version Tauri wrote: newer than 1.1.0.
    assert not older('1.1.0-nightly.202611280930', '1.1.0')
print('Debian release repack preserves root ownership and the versioned migration break, and orders nightlies before the next release')
