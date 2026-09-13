# Getting started

## Install

Download a build from the [releases](https://github.com/wieluk/gameyfin-app/releases) page.

| System | File | Notes |
| --- | --- | --- |
| Windows | `.exe` | Not code signed, so SmartScreen warns on first run. Choose **More info**, then **Run anyway**. |
| Debian, Ubuntu | `.deb` | Needs Ubuntu 24.04 or Debian 13 or newer. Gameyfin tells you when a new version is out; install it the same way. |
| Fedora, openSUSE | `.rpm` | Needs Fedora 40, openSUSE Leap 16 or Tumbleweed, or newer. Updates work as with `.deb`. |
| Any Linux | `.flatpak` | Works on older distributions too, since it brings its own libraries. |

The `.deb` and `.rpm` need glibc 2.39 or newer and WebKitGTK 4.1. On anything older, use the
Flatpak.

### Flatpak: 32-bit support

Installers and older games are 32-bit, and the Flatpak needs Flathub's 32-bit libraries to
run them with Proton. If they are missing, Gameyfin offers **Install 32-bit support** in the
setup step and in **Settings, Compatibility**. Restart Gameyfin afterwards.

## First start

1. **Connect**: enter your Gameyfin server's address.
2. **Sign in**: the server's own login page opens, so SSO works too. If the login window
   misbehaves, use the link under it to clear its saved data.
3. **Choose where games go and name this PC.** The name appears beside every save this PC
   uploads.

On Linux, a setup step then offers to download what Windows games need: Proton, Wine,
DXVK and vkd3d-proton, and on the Flatpak the 32-bit libraries. You can skip it; anything
missing is downloaded on a game's first launch. See [Playing games](playing.md).

## Your games folder

Everything lives in a `Gameyfin` folder inside the games folder you chose:

| Folder | Holds |
| --- | --- |
| `Downloads` | Downloaded archives and unpacked files waiting to install |
| `Installations` | Installed games, named `(id) Title` |
| `Prefixes` | One Windows environment per Windows game (Linux only) |
| `Saves` | Local copies of save backups before they are uploaded |

Because the game id is in the folder name, Gameyfin can find your games again after a
reinstall. Use **Rescan folders** in Installed or Downloads.

You can add more games folders, for example on a second drive, in **Settings, Library**.
