# gameyfin-app

A modern, deeply system-integrated Gameyfin desktop client for Windows and Linux.

Built with Tauri v2, a Rust core for downloads, launching, process supervision and save
sync, with a React/HeroUI interface that shares Gameyfin's design language.

## Planning documents

- [Desktop app development plan](docs/01-desktop-app-plan.md), architecture and the six
  build phases, from scaffolding to packaging.
- [Save sync server plan](docs/02-save-sync-plan.md), the four upstreamable PRs against
  Gameyfin that save game syncing requires.

## Status

Library browsing, downloading, extraction, installation, launching and playtime tracking
work against a live server. Save syncing is designed but not yet wired up.

272 Rust tests and 8 frontend tests pass. Clippy runs with `-D warnings` and the whole
workspace is rustfmt-clean.

## Working offline

The app does not need the server to be reachable to be useful. Installed games are on the
local disk, and an app that shows an empty screen because a router is down is broken.

- The catalogue is mirrored to `catalog.json` in the config directory on every successful
  fetch, and served from there when the server does not answer. Titles may be stale;
  that is better than no library.
- Artwork is read from the on-disk cache before the connection is checked, so a cached
  library renders with its covers rather than as a grid of grey rectangles.
- A session that cannot be *confirmed* is not a session that was *rejected*. An
  unreachable server keeps the stored session, because signing in again is precisely what
  an offline user cannot do; only an actual 401 sends them back to the wizard.
- A banner says which server is unreachable and what will not work until it is back.
  Connection status is re-polled while offline, so the banner clears itself and every
  query is refetched once the server answers again.

Downloading is the one thing that genuinely needs the server, and it fails with a message
saying so. Launching, uninstalling, rescanning and playtime tracking are all local.

## Development

```bash
npm install
npm run dev          # frontend only; falls back to fixture data outside Tauri
npm test
npm run typecheck

cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings

node scripts/fetch-ludusavi.mjs   # bundle the save-backup engine (needed for its tests)
npm run tauri dev                 # the full application
```

## Building installers

```bash
npm run package              # every bundle type for the current platform
npm run package deb rpm      # or just the ones you want
```

Installers are collected into **`build/`** at the project root. Tauri's own output stays in
`target/release/bundle/<type>/`; the script copies from there into one predictable place.

| Platform | Produces | Needs |
|---|---|---|
| Linux | `.deb`, `.rpm`, `.AppImage` | `dpkg-deb`, `rpmbuild`, and `xdg-utils` for AppImage |
| Linux | `.flatpak` | `flatpak`, `flatpak-builder`, GNOME 48 runtime + SDK |
| Windows | `.msi`, `.exe` (NSIS) | WiX and NSIS, installed by the Tauri CLI on first run |

### Flatpak

```bash
flatpak remote-add --if-not-exists --user flathub https://flathub.org/repo/flathub.flatpakrepo
flatpak install --user flathub org.gnome.Platform//48 org.gnome.Sdk//48

npm run package deb          # the Flatpak is assembled from the deb
npm run package:flatpak
```

The manifest lives in [`flatpak/`](flatpak/). It bundles **no Windows runtime** and needs
none installed on the host: the app downloads its own Wine on first use, see
[The Wine runtime](#the-wine-runtime). The host's Wine through `flatpak-spawn --host`
remains as a fallback, which is why `--talk-name=org.freedesktop.Flatpak` is still
requested.

It installs the prebuilt `.deb` rather than compiling in the sandbox, a source build would
need vendored Cargo and npm trees for Flathub's offline builder, which is a lot of machinery
for a build we already do. Flathub submission would require that source build; this targets
self-distribution.

### The Wine runtime

The app downloads a self-contained **Wine-Staging** build (from
[Kron4ek/Wine-Builds](https://github.com/Kron4ek/Wine-Builds)) into its own config
directory and prefers it over anything on the system. One version, identical across the
deb, rpm, AppImage and Flatpak, so a bug report always describes the same runtime, and
nothing to install on the host. That last part matters most on atomic distributions like
Bazzite or Silverblue, where installing Wine means layering an rpm and rebooting.

Detection order, in `detect_windows_runtime_in`: downloaded Wine, then a system `wine`,
then the host's Wine through `flatpak-spawn`, then Proton via umu or Steam.

**Two build variants**, both of which run 32-bit *and* 64-bit Windows programs:

| Variant | 32-bit host libraries |
|---|---|
| `staging-amd64-wow64` (default) | not needed |
| `staging-amd64` | required: multilib, or the i386 Flatpak extension |

Repack installers run 32-bit code, so 32-bit support is not optional. Classic Wine provides
it by loading 32-bit *Linux* libraries; new WoW64 translates inside a pure 64-bit process
and needs none. That is the whole reason for the default: it is the only variant that
behaves identically in all three package formats. New WoW64 is younger than the classic
path, so this is a setting rather than a constant.

#### 32-bit programs need `--allow=multiarch`

Flatpak's seccomp filter admits the i386 architecture only when the `multiarch` feature is
granted, and otherwise blocks `modify_ldt`, which Wine uses to set up the segments 32-bit
code runs in. Without it **64-bit Windows programs run perfectly and every 32-bit one
fails**, which is as misleading a symptom as it sounds: Inno Setup's installer stub is
32-bit, and so is much of the Unity back catalogue.

The failure surfaces as `Application could not be started, or no application associated
with the specified file`, the same message Wine gives for a *missing* file, with an
empty reason, because Wine cannot load its own message resources either. The same Wine
binary, prefix and executable work outside the sandbox, so it reads as a packaging problem
rather than a permission one. `--allow=multiarch` is in `finish-args` for this reason; do
not remove it.

#### Settings that cross a sandbox boundary

When the app falls back to the host's Wine through `flatpak-spawn`, the program is spawned
by `flatpak-session-helper` **on the host**, not forked by us, so nothing set on our own
process reaches it, environment variables and resource limits alike. Both have to travel
inside the command:

- `WINEPREFIX` and friends as `--env=` arguments (`WindowsRuntime::wrap_args`).
- The installer's address-space cap as a `ulimit -v` wrapper
  (`ResolvedCommand::cap_address_space`).

A setting applied to our own process will silently do nothing on that path. Test any new
Wine setting against the Flatpak specifically, not just the deb or rpm. The downloaded Wine
has no such boundary, which is the main reason it is preferred.

#### Prefix layouts

Wine treats the prefix directory as the prefix itself. Proton treats it as a Steam compat
data path and builds the real prefix in `pfx` inside it, so `drive_c` and `dosdevices` sit
one level deeper. Use `prefix::wine_root` rather than assuming either layout.

**Installers cannot be cross-built.** A Windows `.msi` needs WiX running on Windows, and the
Linux bundlers need Linux packaging tools. `.github/workflows/release.yml` builds each
platform on its own runner and attaches both to a draft GitHub release, which is the
supported way to produce a Windows build.

Running `npm run dev` without Tauri shows a banner and fixture data, so the interface can be
worked on without a server.

## Installing

Downloads are attached to each [release](https://github.com/gameyfin/gameyfin-app/releases)
as `.deb`, `.rpm`, `.AppImage` and `.flatpak` for Linux, and `.msi` and `.exe` for Windows.

The Windows installers are **not code signed**, so SmartScreen will warn on first run.
Choose "More info" and then "Run anyway", or build from source.

## Licence

MIT. Ludusavi, bundled as a save-backup sidecar, is MIT licensed; its notice is fetched
alongside the binary by `scripts/fetch-ludusavi.mjs`.
