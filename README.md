# gameyfin-app

A system-integrated Gameyfin desktop client for Windows and Linux. Tauri v2, a Rust core
for downloads, launching, process supervision and save sync, and a React/HeroUI interface.

## Status

Library browsing, downloading, extraction, installation, launching and playtime tracking
work against a live server. Save syncing is designed but not yet wired up. See
[docs/01-desktop-app-plan.md](docs/01-desktop-app-plan.md) and
[docs/02-save-sync-plan.md](docs/02-save-sync-plan.md).

## Development

On Linux the build needs `libudev-dev` (`systemd-devel` on Fedora) plus the usual WebKit
and GTK dev packages.

```bash
npm install
npm run dev          # frontend only, fixture data, no server needed
npm test
npm run typecheck

node scripts/fetch-ludusavi.mjs   # save-backup sidecar, needed for its tests
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
npm run tauri dev                 # the full application
```

## Building installers

```bash
npm run package              # every bundle type for the current platform
npm run package deb rpm      # or just the ones you want
npm run package:flatpak      # assembled from the deb; needs flatpak-builder + GNOME 48 SDK
```

Installers land in `build/`.

| Platform | Produces | Needs |
|---|---|---|
| Linux | `.deb`, `.rpm`, `.AppImage` | `dpkg-deb`, `rpmbuild`, `xdg-utils` |
| Linux | `.flatpak` | `flatpak`, `flatpak-builder`, GNOME 48 runtime + SDK |
| Windows | `.msi`, `.exe` | WiX and NSIS, installed by the Tauri CLI on first run |

Installers cannot be cross-built; `.github/workflows/release.yml` builds each platform on
its own runner.

## Installing

Grab a build from the [releases](https://github.com/gameyfin/gameyfin-app/releases) page.
The Windows installers are not code signed, so SmartScreen warns on first run: choose
"More info" then "Run anyway".

## Licence

MIT. Ludusavi, bundled as a save-backup sidecar, is MIT licensed; its notice is fetched
alongside the binary by `scripts/fetch-ludusavi.mjs`.
