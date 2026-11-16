# Updating

The **Check for updates** button at the top right shows the latest release's notes, from [CHANGELOG.md](../../CHANGELOG.md), and whether there's something newer.

## Mac and Windows

New versions download in the background, checked after launch and every four hours, and install when you quit. To install one straight away, click **Update** in the tab bar. Updates are signed, and Writan refuses one whose signature doesn't match.

## Debian and Ubuntu

Installing the .deb adds Writan's APT repository and its signing key:

```
/etc/apt/sources.list.d/writan.list
/usr/share/keyrings/writan-archive-keyring.gpg
```

After that, new versions come with your system's other updates (Software Updater, or `sudo apt update && sudo apt upgrade`). The **Update** button can also install one: it asks for your password, runs apt for Writan alone, and reopens the app.

Copies installed from 0.7.1 or earlier don't have the repository. Install the newest .deb once by hand and they will. The .deb writes `writan.list` when that file is absent. If an older package wrote `workspace.list`, upgrading moves it to `writan.list` and updates the key path inside it. A `deb` line you commented out stays commented out. If you made the file yourself, check that its `deb` line matches the one below.

To stop updates, put a `#` in front of the `deb` line in `writan.list`. Neither apt nor the **Update** button installs new versions after that (the button only shows the release notes). Upgrading Writan leaves that choice in place. Removing Writan deletes the list, along with the key.

To add the repository by hand (for example on a machine that never had the .deb), install the key from this repository's `src-tauri/linux/writan-archive-keyring.gpg` to `/usr/share/keyrings/` and add:

```
deb [arch=amd64 signed-by=/usr/share/keyrings/writan-archive-keyring.gpg] https://github.com/elijahroizman/writan/releases/download/apt/ ./
```

## Fedora

Installing the .rpm adds Writan's dnf repository and its signing key (the same key as the APT repository):

```
/etc/yum.repos.d/writan.repo
/etc/pki/rpm-gpg/RPM-GPG-KEY-writan
```

After that, new versions come with your system's other updates (GNOME Software, or `sudo dnf upgrade`). The first time, dnf asks you to accept the key. The **Update** button can also install one: it asks for your password, runs dnf for Writan alone, and reopens the app.

The repository's index is signed, and dnf refuses an index whose signature doesn't match. The packages themselves aren't signed; the signed index holds their checksums, so a changed package doesn't install either.

To stop updates, set `enabled=0` in `writan.repo`. If an older package wrote `workspace.repo`, upgrading moves it to `writan.repo` and keeps `enabled=0` if you set it. Removing Writan deletes the file.

## Arch Linux

The release page has a `writan-<version>-1-x86_64.pkg.tar.zst` for Arch Linux and Omarchy on Intel and AMD machines, and a `writan-<version>-1-aarch64.pkg.tar.zst` for ARM ones (Arch Linux ARM, Asahi Linux, Omarchy on Apple silicon). `uname -m` prints which you have. Install it, or a newer one over it, with:

```
sudo pacman -U writan-<version>-1-x86_64.pkg.tar.zst
```

There's no pacman repository, so nothing updates itself: pacman won't offer new versions, and the **Update** button shows the release notes without installing. Download the newer package when the button says there is one. The package depends on `gtk3`, `webkit2gtk-4.1`, `libayatana-appindicator`, `openssl` and `xdotool`, which pacman installs from the official repositories. For spell checking in Word and PowerPoint documents, install a Hunspell dictionary such as `hunspell-en_us`.

## Builds you make yourself

A copy you build from source isn't signed and doesn't update itself. Pull the new code and build again.

The Linux package is named `writan`, and the terminal command is `writan`. `workspace` remains on the PATH and runs `writan`. On Debian and Ubuntu, the repository also contains an empty package named `workspace` that depends on `writan`, so a machine that still has the old package switches over on `apt upgrade`. On Fedora, the rpm replaces the `workspace` package on `dnf upgrade`. The signing key is unchanged. Its installed paths are the ones above.
