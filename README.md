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

363 Rust tests and 24 frontend tests pass. Clippy runs with `-D warnings` and the whole
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

## Updating

There is no single mechanism that works for every package format, because what a package
can do about its own files depends on who owns them.

| Format | How it updates |
|---|---|
| AppImage, `.msi`, `.exe` | Gameyfin replaces itself, using Tauri's signed updater |
| Flatpak | `flatpak update`, from the repository each release publishes |
| `.deb`, `.rpm` | apt or dnf owns the files; Gameyfin only says a version exists |

The **check** works everywhere: it is one request to the GitHub releases API, so every
user is told about a release even where the install has to happen elsewhere. Only the
install step differs.

### Signing key, needed once

The in-place updater verifies a signature, so a maintainer has to generate the pair:

```bash
npm run tauri signer generate -- -w ~/.tauri/gameyfin.key
```

Put the **public** half in `tauri.conf.json` under `plugins.updater.pubkey`, and the
private half in the `TAURI_SIGNING_PRIVATE_KEY` repository secret (with
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` if you set one). Until that is done the release build
still succeeds and simply produces no updater artifacts, and those users fall back to
being told a release exists. **Never commit the private key.**

### The Flatpak repository

`.flatpak` bundles are one-off installs with no remote to check later, so the release
workflow also exports each build into an ostree repository published to GitHub Pages.
Users add it once:

```bash
flatpak remote-add --if-not-exists --user gameyfin \
  https://gameyfin.github.io/gameyfin-app/repo/gameyfin.flatpakrepo
flatpak install --user gameyfin org.gameyfin.Gameyfin
```

After that `flatpak update` and GNOME Software handle it like anything else. The workflow
fetches the existing repository before adding to it, because an ostree repo is a running
history and rebuilding it each release would break the upgrade path from older versions.

## Games folders

More than one folder can be configured, each on its own drive. When there are several you
are asked which to use as a download starts, with the free space on each shown — choosing
where a 90 GB download goes without being told which drive has room for it is not a
choice, and a full disk otherwise surfaces hours later as a failed transfer.

A game is not tied to the default folder: it stays wherever it was sent, and installs,
prefixes and retries all follow the game rather than the current default. Removing a
folder from the list only makes Gameyfin forget it; nothing on disk is touched.

## Per-game options

Installed games take **launch options** and **setup options** — the flags that come from a
wiki page or a ProtonDB report, pasted as written. A silent-install flag under setup
options is what lets an installer-based game be installed unattended.

The box is parsed for quoting only. No variables, globs, pipes or operators: `$HOME` and
`;` are literal text, because it is a box for arguments and treating it as a command line
would make a text field an execution vector. Windows paths keep their backslashes inside
quotes, which is the one place this deliberately differs from a shell.

Settings also carries a list of **executables never to offer** (crash handlers,
redistributables) and a **default archive password**, tried whenever an archive turns out
to be encrypted.

## Archives

Zip, 7z and the tar family (plain, `.gz`, `.xz`, `.zst`, `.bz2`) are unpacked in-process.
The format is read from the file's own bytes, not its name, because a library holds
whatever its owner put there and the extension is often wrong or missing.

**RAR needs a tool on the system.** The only complete RAR implementations carry licence
terms that forbid shipping them inside another application, so Gameyfin uses whatever is
installed and says so when nothing is:

- On Windows, install [7-Zip](https://7-zip.org) or WinRAR. Neither puts itself on `PATH`,
  so Gameyfin looks in the folders they install to as well; nothing needs configuring.
- On Linux, install `unar`, `p7zip` or `unrar` from your distribution.

A **Windows installer that asks for administrator rights** cannot simply be started:
Windows refuses, with `ERROR_ELEVATION_REQUIRED`, and will not let one process quietly
elevate another. Gameyfin reports that as its own thing rather than a failed install and
offers **Run as administrator** on the download, which re-runs the setup program through
the shell so that Windows shows its own consent dialog.

## Playing from outside the app

Installed games can be registered in three places, from the Installed tab:

- The **applications menu** and the **desktop**, as `.desktop` entries.
- **Steam**, as a non-Steam game, so they appear in Big Picture.

All three run `gameyfin-app --launch <id>` rather than the game's executable directly
(inside a Flatpak, `flatpak run org.gameyfin.Gameyfin --launch <id>`, since the binary's
own path means nothing on the host). That indirection is the point: launching through
Gameyfin prepares the Wine prefix, picks the runtime and supervises the session, so
playtime is still recorded. A shortcut straight to the `.exe` would do none of that.

A shortcut used while the app is already open hands its argument to the running copy
rather than starting a second one, which would fight the first over the library file and
the download checkpoints.

Steam's `shortcuts.vdf` is shared with every other launcher that writes to it, so the app
parses the whole file, changes only its own entry, and keeps a backup before writing.

## Controller support

The whole interface can be driven from a pad. `gilrs` reads it on a dedicated thread
(evdev on Linux, XInput on Windows) and the frontend moves **real DOM focus**, which means
keyboard users get the same navigation for free and a focused element scrolls itself into
view. Movement is geometric rather than in document order, because a library grid is a
grid: Down should move a row, not to the next tile.

| Button | Action |
|---|---|
| D-pad / left stick | Move between items |
| A | Select |
| B | Back, or close |
| Y | Refresh |
| LB / RB | Previous / next tab |
| LT / RT | Page up / down |
| Right stick | Scroll |
| Start | Show the button map |

Connecting a pad also switches to a larger layout meant to be read from a sofa; that is a
setting, and the overlay can switch back.

## Elsewhere in the interface

- **Trailers** on the game page. The server has been sending `videoUrls` all along and the
  app discarded them. Nothing is loaded until you click: embedding every trailer on open
  would contact YouTube for each one the moment a game was looked at.
- **Filter by genre, developer or publisher**, alongside the library and installed
  filters. The options are built from what is actually in your library, so a filter never
  offers a value that matches nothing.
- **Light, dark or follow-the-system** themes.
- **Start with your session**, hidden in the tray, which is what makes downloading in the
  background actually work.

## Development

On Linux, controller support links against libudev, so the build needs its development
package (`libudev-dev` on Debian and Ubuntu, `systemd-devel` on Fedora) alongside the
usual WebKit and GTK ones.

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
