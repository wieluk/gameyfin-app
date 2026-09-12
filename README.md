# gameyfin-app

A system-integrated Gameyfin desktop client for Windows and Linux. Tauri v2, a Rust core
for downloads, launching, process supervision and save sync, and a React/HeroUI interface.

## Status

Library browsing, downloading, extraction, installation, launching, playtime tracking and
save syncing all work against a live server. Saves are backed up through a bundled Ludusavi
sidecar and kept on the Gameyfin server, a folder or WebDAV, with conflicts between machines
resolved on restore.

## Development

On Linux the build needs `libudev-dev` (`systemd-devel` on Fedora) plus the usual WebKit
and GTK dev packages.

```bash
npm install
npm run dev          # frontend only, fixture data, no server needed
npm test
npm run typecheck

npm run fetch-sidecars            # the bundled helpers, needed for the Rust tests
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
npm run tauri dev                 # the full application
```

## Building installers

```bash
npm run package              # every bundle type for the current platform
npm run package deb rpm      # or just the ones you want
npm run package:flatpak      # assembled from the deb; needs flatpak-builder + GNOME 50 SDK
```

Installers land in `build/`.

| Platform | Produces | Needs |
|---|---|---|
| Linux | `.deb`, `.rpm` | `dpkg-deb`, `rpmbuild` |
| Linux | `.flatpak` | `flatpak`, `flatpak-builder`, GNOME 50 runtime + SDK |
| Windows | `.exe` | NSIS, installed by the Tauri CLI on first run |

Installers cannot be cross-built; `.github/workflows/release.yml` builds each platform on
its own runner. The Linux packages are built in an Ubuntu 22.04 container: a binary needs
the glibc it was built against or newer, and the runner's own 24.04 would leave them
unable to start on anything older. The Flatpak, which carries its own runtime, is the
answer for distributions the deb and the rpm cannot reach.

## Installing

Grab a build from the [releases](https://github.com/wieluk/gameyfin-app/releases) page.
The Windows installers are not code signed, so SmartScreen warns on first run: choose
"More info" then "Run anyway".

## Licence

MIT. Ludusavi, bundled as a save-backup sidecar, is MIT licensed; its notice is fetched
alongside the binary by `scripts/fetch-sidecar.mjs`.
