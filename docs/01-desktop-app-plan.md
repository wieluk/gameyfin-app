# Gameyfin Desktop App, Development Plan

**Target:** Windows + Linux native client for Gameyfin 2.4.0+
**Stack:** Tauri v2 (Rust core) + React 19 / HeroUI / Tailwind
**Location:** `gameyfin-app/`

---

## Context

Gameyfin's existing desktop client ([Gameyfin-Desktop](https://github.com/gameyfin/Gameyfin-Desktop)) is PyQt6 wrapping a
`QWebEngineView` of the server's web UI. It works, and its native layer is genuinely useful
(umu/Proton prefixes, Steam shortcut registration, stream-extract downloads, gamepad nav), but
the UI *is* the website in a window, so it reads as a browser rather than an application.

GameVault ([gamevault-app](https://github.com/Phalcode/gamevault-app)) shows the bar: a real MVVM desktop app with custom
chrome, a resumable download manager, install/launch pipelines, playtime tracking, offline mode,
Steam integration and ludusavi-backed cloud saves. It is Windows-only and WPF-bound.

We want that class of application, cross-platform, deeply integrated into both Windows and Linux.

### The controlling constraint

Gameyfin's API is almost entirely **Vaadin Hilla RPC**, `POST /connect/<Endpoint>/<method>`,
authenticated by **session cookie + CSRF**, with CORS disabled
(`app/.../core/security/SecurityConfig.kt`). Only three plain REST controllers exist:
`/download/{gameId}`, `/images/**`, and a login redirect.

There is no token or API-key auth. This is exactly why the current client embeds Chromium: it
needs a real browser session to hold cookies and mint CSRF tokens, then proxies RPC calls
*through the page*. Any serious native client either repeats that hack or gets a token endpoint
added server-side. We are doing the latter, see `02-save-sync-plan.md`, PR A.

Until PR A lands, the client ships a fallback cookie-session strategy so development is never
blocked on server review.

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

**Why this split:** everything stateful, privileged or long-running lives in Rust and survives
UI reloads. The webview is a pure view layer that subscribes to Rust events. This is the
structural difference from the old client, where the web page *was* the app.

**Design language:** Gameyfin's web frontend already uses HeroUI + Tailwind
(the server's `app/src/main/frontend/heroui.ts`, with themes in `theming/themes`).
We reuse its tokens for brand continuity, but the layout is app-shaped, persistent sidebar,
custom title bar, no browser affordances, no page reloads.

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
2. **`CookieSessionAuth`** (fallback, ships first), a Tauri webview window loads
   `/login`, we harvest the session cookie once authenticated, and derive the CSRF token the
   way Hilla does (Spring `XSRF-TOKEN` cookie → `_csrf` meta → Vaadin `csrfToken`). This
   mirrors what `Gameyfin-Desktop/gameyfin_frontend/services/gameyfin_api.py` already proves
   works, including SSO/OIDC flows.

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

Mirror the catalog into SQLite so the app opens instantly and works offline (GameVault's
offline mode, done properly with a real database rather than compressed JSON blobs). Cover art
cached to disk, keyed by image id, with blurhash placeholders, the server already returns
`blurhash` on `ImageDto`, so we get progressive loading for free.

### Live updates

Gameyfin pushes changes over Hilla reactive `Flux` subscriptions (`GameEndpoint.subscribe()`,
`LibraryEndpoint.subscribeToLibraryEvents()`), transported by Atmosphere. Subscribe in Rust,
reconcile into SQLite, emit Tauri events to the UI. If the Atmosphere handshake proves awkward
outside a browser, fall back to polling `getAll()` on an interval, the UI contract is
unchanged either way.

**Done when:** log in, browse the full library with artwork, offline, with live updates.

---

## Phase 2, Downloads and install

### Transfer

`GET /download/{gameId}?provider=<key>` streams the file
(`core/download/files/DownloadEndpoint.kt`).

**Known gap:** the endpoint uses `StreamingResponseBody` with no `Accept-Ranges` and no
`Range` handling, so *resume is impossible today*, an interrupted 80 GB download restarts.
PR B fixes this. Client-side we implement it as GameVault does, which works the moment the
server supports it:

- Checkpoint sidecar (`resume_position`, `total_size`, `etag`) written every ~2s.
- On resume, send `Range: bytes=<pos>-`; if the server answers `200` instead of `206`,
  transparently fall back to restarting.
- Verify size on completion; keep `Content-Disposition` filename (server already emits
  RFC 5987 encoded names).

Queue with configurable concurrency (GameVault has no limiter and just spawns parallel
downloads, we do better), pause/resume/cancel, per-drive free-space preflight, exponential
backoff retry, and taskbar/progress integration.

### Extraction and install

- Extract via the `sevenz-rust2` / `zip` / `unrar` crates in Rust rather than shelling to
  `7z.exe`, so Linux gets identical behaviour without a native dependency. Bundle the 7z
  sidecar only as a fallback for exotic formats.
- Stream-extract while downloading where the format allows (the old Python client already does
  this with `stream-unzip`, and it halves peak disk usage), otherwise extract on completion.
- Encrypted archives: detect and prompt for a password.
- Layout: `<root>/Gameyfin/Downloads/(<id>) <Title>/` and
  `<root>/Gameyfin/Installations/(<id>) <Title>/`. The `(id)` prefix is GameVault's trick for
  recovering game identity from a bare directory, worth keeping.
- Install record in SQLite (not a file in the game folder), plus a small
  `gameyfin-install.json` in the directory so a moved/copied install is still recognisable.
- Executable detection: walk for candidates, score them (root-level, name similarity to title,
  size, not in `redist/`/`_CommonRedist`/`DirectX`), auto-pick a confident single match,
  otherwise prompt. Remember the choice; allow override in game settings.
- Windows installers (`setup.exe`): run with `%INSTALLDIR%` templating, as GameVault does.

**Done when:** download → extract → install → correct executable is detected, with resume
surviving an app restart.

---

## Phase 3, Launch, playtime, process supervision

### Launching

- **Windows:** spawn directly, working directory set to the executable's folder.
- **Linux, native games:** spawn directly.
- **Linux, Windows games:** **Wine**, with a **per-game Wine prefix** at
  `~/.local/share/gameyfin/prefixes/<id>/`.

  *Revised from the original plan, which called for `umu-run` with GE-Proton throughout,
  what the current Python client does (`services/game_launcher.py`, `umu_database.py`) and
  what Heroic and Lutris do.* Proton has better per-title compatibility, but it arrives
  through umu or Steam, each with its own installation, download and failure modes, and umu
  inside a Flatpak would have to bring a Python stack with it. Wine is packaged by every
  distribution and is the same runtime across the deb, rpm, AppImage and Flatpak, which
  means one thing for a user to install and one thing to diagnose. Proton is still used when
  Wine is absent and umu or a Steam build happens to be there; the umu id and GE-Proton
  handling remain in `Runtime::Proton` for that path.
- Per-game launch config: environment variables, Proton version, prefix, launch arguments,
  gamescope/MangoHud toggles on Linux.

### Process supervision, beating GameVault

GameVault polls every 60 seconds, enumerating *every* process on the system and string-matching
`MainModule.FileName` against install directories, with a user-maintained ignore list for
launcher subprocesses (`Helper/GameTimeTracker.cs`). Playtime is therefore quantised to the
minute and misattributed whenever a game spawns a launcher that exits.

We do it properly, event-driven:

- **Windows:** assign the launched process to a **Job Object** with
  `JOB_OBJECT_ALL_ACCESS`, and wait on an IO completion port for
  `JOB_OBJECT_MSG_ACTIVE_PROCESS_ZERO`. This captures the entire process tree, including
  games that relaunch themselves through a bootstrapper, and tells us exactly when the last
  descendant exits.
- **Linux:** launch in a new process group / `PR_SET_CHILD_SUBREAPER`, and reap the group.
  For `umu-run`, wait on the umu wrapper process which already owns the Proton tree.

This gives exact wall-clock playtime, a reliable "game exited" signal (which is what triggers
save backup), and no ignore list.

Playtime is written to SQLite immediately and synced to the server via PR D, buffering while
offline and flushing on reconnect.

**Done when:** launching a game shows accurate live playtime, and exit is detected within a
second even for bootstrapper-based titles.

---

## Phase 4, Save game sync

Full protocol and server side in `02-save-sync-plan.md`. Client responsibilities:

### Driving ludusavi

Bundled as a Tauri sidecar, invoked with an **isolated config directory** via the global
`--config <dir>` flag so we never touch the user's own `~/.config/ludusavi`. We write that
config ourselves (roots, redirects, backup path, custom games).

Integration is by **subprocess + `--api` JSON**, not the Rust `lib` target: ludusavi's
`src/lib.rs` carries an explicit "the API will be unstable" warning, whereas the CLI's JSON
contract is documented and schema-generated (`docs/schema/general-output.yaml`). Version pinning
plus a stable contract beats tight coupling here.

**We do not use `ludusavi wrap`.** It blocks the caller until the game exits and owns process
launching as a black box, which is incompatible with Phase 3's supervision (we need the PID,
the exit signal, and control over the tree). Manual `restore` before launch and `backup` after
exit gives identical behaviour with full control.

### Game matching

This is where GameVault is weakest: it runs `find <title> --fuzzy --api` and accepts a match if
the score exceeds 0.9. Ludusavi's own precedence is **Steam ID → GOG ID → exact name →
normalized name**, and ID lookup is deterministic.

Gameyfin already stores exactly what we need. The Steam metadata plugin writes the **Steam
AppID** as `originalId` (`plugins/steam/.../SteamPlugin.kt:212`), kept in
`GameMetadata.originalIds: Map<PluginManagementEntry, String>`.

**But `GameMetadataUserDto` exposes only `fileSize`**, `originalIds` is admin-only
(`games/dto/GameMetadataDto.kt`). So exposing external IDs to normal users is a necessary
server change (PR C1, three lines). With it, our resolution ladder becomes:

1. `find --steam-id <appid> --api`, exact, when the game was matched by the Steam plugin.
2. `find "<title>" --normalized --api`, forgiving on edition/year suffixes.
3. `find "<title>" --fuzzy --multiple --api`, last resort, and we **ask the user to confirm**
   rather than silently trusting a score.
4. Manual override, plus a ludusavi `customGames` entry for titles absent from the manifest.

The resolved title is cached in SQLite and mirrored to the server so every device agrees.

### Backup and restore

```
ludusavi --config <appcfg> backup  --force --api --format zip --compression zstd \
                                   --path <staging> "<Title>"
ludusavi --config <appcfg> restore --force --api --path <staging> "<Title>"
```

The sync unit is **one game folder**, `mapping.yaml` plus the zip(s) it indexes. `mapping.yaml`
is authoritative (per-file hashes and sizes, backup timestamps) and restore ignores folders
without it, so it must be included in the uploaded bundle.

### Cross-machine portability, the redirects problem

GameVault makes saves portable by writing `redirects` that map the user profile and the install
directory onto fixed fake paths (`G:\gamevault\currentuser`, `G:\gamevault\installation`), so a
save taken on one PC restores on another with a different username. That trick is Windows-only.

Our config generalises it:

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

The `<game>` placeholder in a root path lets one entry cover our whole per-game prefix
collection without globbing every prefix for every game.

**Windows ↔ Linux is the honest caveat.** Ludusavi does not translate save locations across
operating systems (`docs/help/transfer-between-operating-systems.md`); the experimental
`scan.redirectWine` is unfinished. However, a Windows game run under Proton on Linux keeps its
Windows-shaped paths *inside the prefix*, so **Windows ↔ Proton sync works** through the
redirects above, which covers the overwhelmingly common case for a Gameyfin library. We tag
each save with its platform and warn, rather than silently corrupting, on a genuine
Windows↔native-Linux mismatch.

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

**Both platforms:** system tray with quick-launch, native notifications, global settings,
single-instance with deep links (`gameyfin://game/<id>`), gamepad navigation (the current
Python client's full-app gamepad support is a genuinely nice feature worth keeping, `gilrs`
crate), and auto-update via Tauri's updater.

**Windows:** custom chrome with proper snap/aero behaviour, taskbar progress and jump list of
installed games, Start Menu and desktop shortcuts, MSI/NSIS installer, optional Start-with-Windows.

**Linux:** `.desktop` entries in `~/.local/share/applications` with icons, MPRIS-style tray via
`ksni`, Flatpak (using the **host's Wine** through `flatpak-spawn`, rather than bundling a
runtime) plus AppImage and `.deb`/`.rpm`, and XDG base-directory compliance.

**Both:** register installed games as **non-Steam shortcuts** in
`~/.local/share/Steam/userdata/<id>/config/shortcuts.vdf` (and the Windows equivalent) so they
appear in Steam Big Picture / Steam Deck gaming mode. The current Python client already
implements this correctly across native/Flatpak/legacy Steam locations
(`services/steam_integration.py`), port that logic.

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

Rust: unit tests per module, `mockito` for the Gameyfin API, and a fixture ludusavi output
corpus so the JSON parsing is tested without the binary. Frontend: Vitest + Testing Library.
End-to-end smoke test against a real Gameyfin instance in Docker (the server repo's `docker/` compose setup).

---

## Sequencing note

Phases 1-3 depend on **no** server changes (using the cookie fallback). Phase 4 depends on
PR A + PR C. So client work can start immediately and in parallel with server review, which is
the main reason for the auth strategy trait in Phase 1.
