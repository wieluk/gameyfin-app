# Gameyfin Desktop App, Development Plan

**Target:** Windows + Linux native client for Gameyfin 2.4.0+
**Stack:** Tauri v2 (Rust core) + React 19 / HeroUI / Tailwind
**Location:** `gameyfin-app/`

---

## Context

The existing client ([Gameyfin-Desktop](https://github.com/gameyfin/Gameyfin-Desktop)) is PyQt6 around a `QWebEngineView`;
its native layer is useful but the UI *is* the website. [gamevault-app](https://github.com/Phalcode/gamevault-app) shows the
bar (custom chrome, resumable downloads, install/launch pipelines, playtime, offline, Steam,
ludusavi saves) but is Windows-only WPF. We want that class of app, cross-platform.

### The controlling constraint

Gameyfin's API is almost entirely **Vaadin Hilla RPC** (`POST /connect/<Endpoint>/<method>`),
authenticated by **session cookie + CSRF**, CORS disabled. Only `/download/{gameId}`,
`/images/**` and a login redirect are plain REST. There is no token auth, which is why the
current client embeds Chromium. We add a token endpoint server-side (`02-save-sync-plan.md`
PR A) and ship a cookie-session fallback until it lands.

---

## Architecture

```
gameyfin-app/
├─ src-tauri/
│  ├─ src/
│  │  ├─ main.rs
│  │  ├─ api/            Gameyfin client: auth strategies, Hilla RPC, REST, SSE
│  │  ├─ db/             SQLite (sqlx), catalog cache, installs, downloads, saves, playtime
│  │  ├─ download/       resumable transfers, stream-extract, queue + scheduler
│  │  ├─ install/        archive handling, exe detection, install records
│  │  ├─ launcher/       Windows direct exec; Linux umu-run + per-game Proton prefixes
│  │  ├─ process/        event-driven supervision (Job Objects / process groups)
│  │  ├─ saves/          ludusavi driver + sync engine + conflict resolution
│  │  ├─ integration/    Steam shortcuts.vdf, desktop entries, tray, notifications
│  │  └─ ipc/            typed Tauri commands + event channels
│  └─ binaries/          bundled ludusavi (+ 7z on Windows) as Tauri sidecars
└─ src/                  React 19 + HeroUI + Tailwind
   ├─ views/             Library, Game, Downloads, Installed, Settings
   ├─ components/
   └─ state/             TanStack Query + Zustand
```

**Why this split:** everything stateful, privileged or long-running lives in Rust and
survives UI reloads; the webview is a pure view layer subscribing to Rust events.

**Design language:** reuse Gameyfin's HeroUI + Tailwind tokens for brand continuity, but
an app-shaped layout: persistent sidebar, custom title bar, no browser affordances.

---

## Phase 0, Scaffolding

- `pnpm create tauri-app` → React + TypeScript + Vite; Tauri v2.
- Workspace: `src-tauri` as a Cargo workspace so modules are separately testable crates.
- Configure Tauri capabilities narrowly (no blanket `fs:default`); shell access limited to
  declared sidecars.
- CI: GitHub Actions matrix (windows-latest, ubuntu-latest) building on every push.
- Vendor `ludusavi` binaries per target triple into `src-tauri/binaries/` as Tauri sidecars
  (MIT licensed, bundling permitted; ship the notice).

**Done when:** an empty window builds and runs on both OSes in CI.

---

## Phase 1, Auth and library

### Auth strategy trait

```rust
#[async_trait]
trait AuthStrategy {
    async fn authenticate(&self, req: RequestBuilder) -> Result<RequestBuilder>;
    async fn refresh(&self) -> Result<()>;
}
```

Two implementations:

1. **`DeviceTokenAuth`** (target), `Authorization: Bearer <token>` against the new
   `/api/auth/device` endpoints from PR A. Token stored in the OS keychain
   (`keyring` crate: Credential Manager / libsecret).
2. **`CookieSessionAuth`** (fallback, ships first), a webview window loads `/login`, we
   harvest the session cookie and derive the CSRF token as Hilla does (Spring `XSRF-TOKEN`
   cookie → `_csrf` meta → Vaadin `csrfToken`). Proven by the Python client, SSO included.

The client is written against the trait, so PR A landing is a config change, not a rewrite.

### Hilla RPC client

Thin typed wrapper over `POST /connect/<Endpoint>/<method>` with a JSON body of named params.
Endpoints needed for v1 (all `@DynamicPublicAccess @AnonymousAllowed`, so readable by any
authenticated user):

| Endpoint | Method | Use |
|---|---|---|
| `GameEndpoint` | `getAll()` | full catalog |
| `LibraryEndpoint` | `getAll()` | library grouping |
| `CollectionEndpoint` | `getAll()` | collections |
| `DownloadProviderEndpoint` | `getProviders()` | pick download source by priority |
| `UserEndpoint` | `getUserInfo()` | current user identity |
| `UserPreferencesEndpoint` | `get/set(key)` | theme, favourites (until PR D) |

### Local catalog

Mirror the catalog to disk so the app opens instantly and works offline. Cover art cached
by image id, with `blurhash` placeholders the server already returns.

### Live updates

Subscribe to Hilla `Flux` subscriptions in Rust, reconcile locally, emit Tauri events. Fall
back to polling `getAll()` if the Atmosphere handshake is awkward outside a browser.

**Done when:** log in, browse the full library with artwork, offline, with live updates.

---

## Phase 2, Downloads and install

### Transfer

`GET /download/{gameId}?provider=<key>` streams the file
(`core/download/files/DownloadEndpoint.kt`).

**Known gap:** the endpoint has no `Accept-Ranges`/`Range` handling, so resume is impossible
until PR B. Client-side we implement it anyway so it works the moment the server supports it:

- Checkpoint sidecar (`resume_position`, `total_size`, `etag`) written every ~2s.
- On resume send `Range: bytes=<pos>-`; fall back to restarting if the server answers `200`.
- Verify size on completion; keep the `Content-Disposition` filename.

Queue with configurable concurrency, pause/resume/cancel, per-drive free-space preflight,
backoff retry, taskbar progress.

### Extraction and install

- Extract via Rust crates, not a shelled `7z.exe`, so Linux matches without a native dep;
  7z sidecar as a fallback for exotic formats.
- Stream-extract while downloading where the format allows, otherwise on completion.
- Encrypted archives: detect and prompt for a password.
- Layout: `<root>/Gameyfin/Downloads/(<id>) <Title>/` and `.../Installations/(<id>) <Title>/`;
  the `(id)` prefix recovers game identity from a bare directory.
- Install record kept locally plus a `gameyfin-install.json` in the directory so a
  moved install is still recognisable.
- Executable detection: score candidates (root-level, title similarity, size, not in
  redist dirs), auto-pick a confident match else prompt, remember the choice.
- Windows installers (`setup.exe`): run with `%INSTALLDIR%` templating.

**Done when:** download → extract → install → correct executable is detected, with resume
surviving an app restart.

---

## Phase 3, Launch, playtime, process supervision

### Launching

- **Windows:** spawn directly, working directory set to the executable's folder.
- **Linux, native games:** spawn directly.
- **Linux, Windows games:** **Wine**, with a **per-game Wine prefix** at
  `~/.local/share/gameyfin/prefixes/<id>/`.

  *Revised from the original umu/GE-Proton plan.* Proton has better per-title compat but
  arrives through umu or Steam, each with its own failure modes, and umu in a Flatpak needs
  a Python stack. Wine is one runtime across every package format. Proton stays as a
  fallback (`Runtime::Proton`, with umu id and GE-Proton handling) when Wine is absent.
- Per-game launch config: environment variables, Proton version, prefix, launch arguments,
  gamescope/MangoHud toggles on Linux.

### Process supervision

GameVault polls every 60s, string-matching every process against install dirs with a
user-maintained ignore list. We do it event-driven:

- **Windows:** a **Job Object** waited on via IO completion port for
  `JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO`, capturing the whole tree including bootstrappers.
- **Linux:** a new process group / `PR_SET_CHILD_SUBREAPER`, reaping the group; for
  `umu-run`, wait on the wrapper that owns the Proton tree.

Exact wall-clock playtime, a reliable exit signal (which triggers save backup), no ignore
list. Playtime is stored immediately and synced via PR D, buffering while offline.

**Done when:** launching a game shows accurate live playtime, and exit is detected within a
second even for bootstrapper-based titles.

---

## Phase 4, Save game sync

Full protocol and server side in `02-save-sync-plan.md`. Client responsibilities:

### Driving ludusavi

Bundled as a Tauri sidecar, invoked with an **isolated config directory** via the global
`--config <dir>` flag so we never touch the user's own `~/.config/ludusavi`. We write that
config ourselves (roots, redirects, backup path, custom games).

Integration is by **subprocess + `--api` JSON**, not the Rust `lib` target, which is
explicitly unstable. We do **not** use `ludusavi wrap`: it owns process launching as a black
box, incompatible with Phase 3's supervision. Manual `restore` before, `backup` after.

### Game matching

Ludusavi's precedence is **Steam ID → GOG ID → exact name → normalized name**; ID lookup is
deterministic. The Steam plugin writes the AppID as `originalId`, but `GameMetadataUserDto`
exposes only `fileSize`, so exposing external IDs to normal users is PR C1. The ladder:

1. `find --steam-id <appid> --api` when matched by the Steam plugin.
2. `find "<title>" --normalized --api`, forgiving on edition/year suffixes.
3. `find "<title>" --fuzzy --multiple --api`, last resort, **with user confirmation**.
4. Manual override plus a `customGames` entry for titles absent from the manifest.

The resolved title is cached and mirrored to the server so every device agrees.

### Backup and restore

```
ludusavi --config <appcfg> backup  --force --api --format zip --compression zstd \
                                   --path <staging> "<Title>"
ludusavi --config <appcfg> restore --force --api --path <staging> "<Title>"
```

The sync unit is **one game folder**, `mapping.yaml` (authoritative: per-file hashes, sizes,
timestamps) plus the zip(s) it indexes. Restore ignores folders without it.

### Cross-machine portability

GameVault makes saves portable with `redirects` mapping the profile and install dir onto
fixed fake paths, so a save restores under a different username. Windows-only; we generalise:

```yaml
roots:
  - { store: steam,      path: "~/.steam/steam" }            # Proton compatdata auto-detected
  - { store: otherWine,  path: "~/.local/share/gameyfin/prefixes/<game>" }
backup:
  path: "<staging>"
  format:    { chosen: zip, zip: { compression: zstd } }
  retention: { full: 1, differential: 0 }                    # server owns versioning
redirects:
  - { kind: bidirectional, source: "<actual home>",        target: "/gameyfin/home" }
  - { kind: bidirectional, source: "<actual install dir>", target: "/gameyfin/install" }
```

The `<game>` placeholder lets one root entry cover the whole per-game prefix collection.

**Windows ↔ Linux caveat.** Ludusavi does not translate save locations across OSes. But a
Windows game under Proton keeps Windows-shaped paths inside its prefix, so **Windows ↔ Proton
works**, which covers the common case. Saves are platform-tagged; a genuine
Windows↔native-Linux mismatch warns rather than corrupting.

### When sync runs

- **Before launch:** check the server for a newer save; restore if the local base hash differs.
- **After exit:** back up, hash, upload if changed. The exit signal comes from Phase 3's job
  object, so this is exact, not GameVault's once-a-minute poll.
- Conflicts surface as a real dialog (keep local / keep remote / keep both), driven by the
  server's `409` response described in the sync plan.

**Done when:** a save made on one machine restores on another, and a genuine conflict produces
a choice rather than silent data loss.

---

## Phase 5, Deep desktop integration

**Both platforms:** tray with quick-launch, notifications, single-instance with deep links
(`gameyfin://game/<id>`), gamepad navigation (`gilrs`), Tauri auto-update.

**Windows:** custom chrome with snap/aero, taskbar progress and jump list, Start Menu and
desktop shortcuts, MSI/NSIS, optional Start-with-Windows.

**Linux:** `.desktop` entries, tray via `ksni`, Flatpak plus AppImage and `.deb`/`.rpm`,
XDG compliance.

**Both:** register installed games as non-Steam shortcuts in `shortcuts.vdf` (parsing the
whole file, changing only our entry) so they appear in Steam Big Picture.

---

## Phase 6, Packaging and release

- Windows: MSI + NSIS, signed if a certificate is available.
- Linux: Flatpak (primary, mirroring `org.gameyfin.Gameyfin-Desktop`), AppImage, `.deb`, `.rpm`.
- Tauri updater with signed manifests; GitHub Releases as the channel.
- Bundle and pin `ludusavi`; ship its MIT notice.

---

## Verification

| Phase | Check |
|---|---|
| 0 | `cargo tauri build` succeeds on both OSes in CI |
| 1 | Log in (password **and** OIDC), browse full library, kill network → catalog still renders |
| 2 | Start a 5 GB download, kill the app mid-transfer, reopen → resumes from checkpoint (needs PR B; without it, verify graceful restart) |
| 3 | Launch a bootstrapper-based game; confirm exit detected within 1s and playtime matches wall clock |
| 4 | Save on machine A → restore on machine B; force a conflict by playing offline on both → dialog appears |
| 5 | Game appears in Steam Big Picture; tray, notifications and deep links work on both OSes |

Rust: per-module unit tests, `mockito` for the API, a fixture ludusavi corpus. Frontend:
Vitest. End-to-end smoke test against a real Gameyfin instance in Docker.

---

## Sequencing note

Phases 1-3 need no server changes (cookie fallback); Phase 4 needs PR A + PR C. Client work
runs in parallel with server review, which is why Phase 1 has the auth strategy trait.
