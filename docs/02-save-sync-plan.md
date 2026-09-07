# Save Game Sync in Gameyfin, Minimal Server Changes

**Target:** [gameyfin/gameyfin](https://github.com/gameyfin/gameyfin) (Kotlin, Spring Boot 4, Vaadin Hilla 25, H2 + Flyway)
**Shape:** four small, independently reviewable, upstreamable PRs

---

## Context

The desktop client needs to back up and restore game saves through the Gameyfin server. This
document specifies the smallest set of server changes that makes that possible, ordered so each
PR stands alone and is useful on its own merits.

### Why this cannot be a plugin

Gameyfin has a PF4J plugin system, and a save-sync plugin would be the ideal blast radius. It is
not possible. `plugin-api/` exposes exactly two extension points, `GameMetadataProvider` and
`DownloadProvider`. Plugins are loaded by `GameyfinPluginManager` into a PF4J classloader that
Spring never component-scans and Hibernate never entity-scans, so a plugin **cannot** contribute
a REST controller, a Hilla endpoint, a JPA entity or a repository. The only persistence a plugin
gets is a private JSON state file and host-owned key/value config.

Save sync therefore has to be core server work. The plan below keeps it contained.

### Why existing state cannot be reused

There is no per-user, per-game state in Gameyfin at all, no playtime, no last-played, no
progress. The nearest thing is `UserPreference`, a generic encrypted key/value table where
`FavouriteGames` and `RecentDownloads` are stored as a **single serialized string per user**.
That is not joinable, not queryable, and obviously not a place for binary save data.

New entities are unavoidable. Schema is H2 + **Flyway** with `ddl-auto: validate`, so every new
table needs a hand-written migration under `app/src/main/resources/db/migration/`.

---

## PR A, Device token authentication

**Problem.** Auth is session-cookie + CSRF only, CORS disabled. There is no way for a native
client to authenticate. This is why the current desktop client embeds Chromium purely to hold a
session, a workaround every third-party client has to reinvent.

**Change.** Reuse the existing generic token infrastructure. `core/token/` already has
`Token<T : TokenType>` (encrypted secret, creator, payload map, creation timestamp, expiry) and
an abstract `TokenService<T>` with `generate` / `generateWithPayload` / `get` / `delete`. Adding
a type is idiomatic and small.

```kotlin
// core/token/TokenType.kt
data object Device : TokenType("device", Duration.INFINITE)
```

New `DeviceTokenService : TokenService<TokenType.Device>`, and a REST controller:

| Route | Method | Purpose |
|---|---|---|
| `/api/auth/device/authorize` | POST | start pairing, returns a short user code + poll handle |
| `/api/auth/device/token` | POST | client polls; returns the bearer token once approved |
| `/api/auth/device/revoke` | DELETE | revoke this device |
| `/api/auth/devices` | GET | list the user's paired devices |

Use the **OAuth 2.0 Device Authorization Grant** shape (RFC 8628): the client shows a code, the
user approves it in their already-authenticated browser session. This avoids ever handling the
user's password in the client, works unchanged with OIDC/SSO (which the current cookie hack
struggles with), and is a familiar pattern for anyone reviewing it.

Then a `DeviceTokenAuthenticationFilter` in the `SecurityConfig` chain accepting
`Authorization: Bearer <secret>`, resolving to the token's creator, ahead of the existing
session auth. Hilla endpoints keep working unchanged, they just see an authenticated principal.

`payload` on the token carries device name, platform and last-seen, so the device list is
useful.

**Files:** `core/token/TokenType.kt`, new `core/token/DeviceTokenService.kt`, new
`core/security/DeviceTokenAuthenticationFilter.kt`, `core/security/SecurityConfig.kt`, new
`core/security/DeviceAuthController.kt`, Flyway migration if the token table needs a payload
index.

**Standalone value:** any third-party client, script or mobile app becomes possible. This is
worth upstreaming regardless of save sync.

---

## PR B, Range support on game downloads

**Problem.** `core/download/files/DownloadEndpoint.kt` returns a `StreamingResponseBody` with no
`Accept-Ranges` header and no `Range` request handling. An interrupted download of a large game
restarts from zero. GameVault has had resumable downloads for years.

**Change.** Where the resolved `Download` is a `FileDownload` with a known `size`, parse the
`Range` header, emit `206 Partial Content` with `Content-Range` and `Accept-Ranges: bytes`, and
skip to the offset before streaming. Where size is unknown or the provider yields a
`LinkDownload`, behave exactly as today.

Keep it conservative: single-range requests only (`bytes=N-` and `bytes=N-M`), respond `416` on
an unsatisfiable range, and leave the existing bandwidth monitoring and download-count logic
untouched.

**Files:** `core/download/files/DownloadEndpoint.kt`, `core/download/files/DownloadService.kt`.

**Standalone value:** fixes resumable downloads for *every* client including browsers.

---

## PR C, Save sync

The core of the feature. Three pieces: expose external IDs, add the entity, add the API.

### C1, Expose external IDs to users (three lines)

Reliable ludusavi matching needs the Steam AppID. Gameyfin already stores it, the Steam plugin
writes it as `originalId` (`plugins/steam/.../SteamPlugin.kt:212`) into
`GameMetadata.originalIds`. But `GameMetadataUserDto` exposes only `fileSize`; `originalIds` is
admin-only.

```kotlin
data class GameMetadataUserDto(
    override val fileSize: Long,
    val originalIds: Map<String, String>? = null,   // pluginId -> external id
) : GameMetadataDto
```

Populate it in the user-DTO mapper (`games/extensions/`). These are public catalogue
identifiers (Steam AppIDs, IGDB ids), no privacy concern.

Without this, the client is stuck with fuzzy title matching and its false positives. With it,
matching is deterministic for any Steam-matched game.

### C2, Entity and migration

Save archives are blobs; they do not belong in H2. Store metadata in the database and bytes on
disk, exactly as the server already does for game files.

```kotlin
@Entity
@Table(name = "game_saves")
class GameSave(
    @Id @GeneratedValue val id: Long? = null,
    @ManyToOne(optional = false) @OnDelete(CASCADE) val user: User,
    @ManyToOne(optional = false) @OnDelete(CASCADE) val game: Game,
    val installationId: String,        // UUID of the client install that produced it
    val contentHash: String,           // SHA-256 of the archive
    val sizeBytes: Long,
    val platform: SavePlatform,        // WINDOWS | LINUX | PROTON
    val ludusaviTitle: String?,        // resolved manifest title, shared across devices
    val locked: Boolean = false,       // exempt from retention pruning
    @CreationTimestamp val createdAt: Instant? = null,
)
```

On-disk layout, mirroring GameVault's proven scheme but adding the hash:

```
<data-dir>/saves/<userId>/<gameId>/<epochMillis>_<installationId>.zip
```

Flyway migration `V2.5.0.1__Create_game_saves_table.sql` creating the table plus indexes on
`(user_id, game_id, created_at DESC)` and `(user_id, game_id, content_hash)`.

### C3, API

Binary transfer does not fit Hilla RPC (JSON-only), so blobs go over plain REST, the same
choice the codebase already made for `DownloadEndpoint` and `ImageEndpoint`. Metadata and
management additionally get a Hilla endpoint so the **web UI** can list and delete saves, which
makes the feature discoverable rather than client-only.

REST, `SaveSyncController`, `/api/saves`:

| Route | Method | Notes |
|---|---|---|
| `/game/{gameId}` | GET | list version metadata (JSON) |
| `/game/{gameId}/latest` | GET | stream newest archive |
| `/game/{gameId}/{saveId}` | GET | stream a specific version |
| `/game/{gameId}` | POST | multipart upload |
| `/game/{gameId}/{saveId}` | DELETE | delete one version |
| `/game/{gameId}` | DELETE | delete all versions for this user |

Upload headers: `X-Installation-Id` (UUID v4), `X-Content-Hash` (SHA-256), `X-Save-Platform`,
and `X-Base-Save-Id`, the id of the version the client last synced from.

Hilla, `SaveSyncEndpoint`, `@PermitAll`: `getSavesForGame(gameId)`, `deleteSave(saveId)`,
`getStorageUsage()`, and an admin `getAllUsage()`.

Authorization: a user may only touch their own saves; admins may delete any. Follow the existing
`SecurityUtils.getCurrentAuth()` pattern.

### C4, Conflict detection (where we beat GameVault)

GameVault's server is pure last-write-wins: `findSavefilesByUserIdAndGameIdOrFail` sorts by
filename timestamp and always serves the newest. The client's only defence is comparing the
installation id embedded in the filename, which tells it a save came from *a different machine*
but nothing about whether it would lose data. Play offline on two PCs and one save is silently
gone.

We add optimistic concurrency, at the cost of one header:

- The client sends `X-Base-Save-Id`: the version it restored from.
- If the newest server-side save for that (user, game) is **not** that id, the server responds
  `409 Conflict` with both versions' metadata (timestamps, sizes, hashes, platforms, device
  names) instead of accepting the upload.
- The client then presents a real choice: keep local, keep remote, or keep both (upload with
  `X-Force: true`, which retains the loser as a normal version rather than deleting it).
- Identical `X-Content-Hash` short-circuits to `204 No Content`, no upload, no new version.
  This alone removes most redundant traffic, since most sessions do not change every save file.

Retention: after a successful upload, prune to `MaxVersionsPerGame` newest, skipping `locked`.

### C5, Configuration

Declaring a `ConfigProperties` object is all that is needed, `ConfigEndpoint.getAll()`
reflectively collects them and the admin UI renders the right control automatically, with **zero
frontend code**.

```kotlin
sealed class SaveSync {
    data object Enabled           : ConfigProperties<Boolean>(..., "save-sync.enabled", default = false)
    data object MaxSizeBytes      : ConfigProperties<Long>(...,   "save-sync.max-size", default = 500L * 1024 * 1024)
    data object MaxVersionsPerGame: ConfigProperties<Int>(...,    "save-sync.max-versions", default = 10)
    data object MaxTotalPerUser   : ConfigProperties<Long>(...,   "save-sync.max-total-per-user", default = 10L * 1024 * 1024 * 1024)
}
```

Disabled by default, matching GameVault (`SAVEFILES_ENABLED` defaults false). When disabled the
routes return `405`, which is the status GameVault's client already understands, a small
courtesy for ecosystem consistency.

**Files:** new package `app/src/main/kotlin/org/gameyfin/app/saves/` (entity, repository,
service, REST controller, Hilla endpoint, DTOs), `config/ConfigProperties.kt`,
`games/dto/GameMetadataDto.kt`, `games/extensions/`, and the Flyway migration.

---

## PR D, Per-user game state

Needed for the client's playtime and favourites, and independently valuable: it replaces the
`FavouriteGames` string blob with something queryable.

```kotlin
@Entity
@Table(name = "user_game_state", uniqueConstraints = [UniqueConstraint(columnNames = ["user_id", "game_id"])])
class UserGameState(
    @Id @GeneratedValue val id: Long? = null,
    @ManyToOne val user: User,
    @ManyToOne val game: Game,
    var minutesPlayed: Int = 0,
    var lastPlayedAt: Instant? = null,
    var favorite: Boolean = false,
    var completed: Boolean = false,
)
```

Hilla `UserGameStateEndpoint` (`@PermitAll`): `getAll()`, `getForGame(gameId)`,
`addPlaytime(gameId, minutes)`, `setFavorite(gameId, Boolean)`, plus a `subscribe(): Flux<...>`
following the established `Sinks.many().multicast()` pattern used by `GameService` and friends.

Additive playtime (`addPlaytime`) rather than absolute assignment, so an offline client can
flush buffered minutes without clobbering another device's progress.

Migration `V2.5.0.2__Create_user_game_state_table.sql`, plus an optional one-off backfill from
the existing `FavouriteGames` preference string.

---

## Sequencing

| PR | Depends on | Client unblocked |
|---|---|---|
| A, device tokens |, | native auth without embedded Chromium |
| B, Range support |, | resumable downloads |
| C, save sync | A | Phase 4 |
| D, user game state | A | playtime, favourites |

A and B are independent and can go up immediately; both are defensible on their own merits
without mentioning the desktop client, which makes them easy to review. C is the substantive
one. D is optional for a first release, the client can hold playtime locally until it lands.

---

## Verification

**Server**
- Unit tests per service, following the existing MockK conventions in `app/src/test/kotlin`.
- `SaveSyncService`: retention pruning respects `locked`; quota enforcement; hash short-circuit.
- `SaveSyncController` with MockMvc: authorization (user A cannot read user B's saves), `409` on
  a stale `X-Base-Save-Id`, `204` on an identical hash, `405` when disabled, `413` over quota.
- `DownloadEndpoint` with MockMvc: `206` with correct `Content-Range`, `416` unsatisfiable,
  unchanged `200` when no `Range` is sent.
- Flyway migrations apply cleanly against a 2.4.0 database, and `ddl-auto: validate` passes at
  boot, this is the check that catches a mismatch between the SQL and the entity.

**End to end**
- Run the server from its repo's `docker/` compose setup.
- Pair a device: confirm the code flow works with local login *and* with OIDC.
- Upload a save, list versions, download it back, verify the SHA-256 round-trips.
- Upload 12 versions with `MaxVersionsPerGame = 10`; confirm the two oldest unlocked are pruned
  and a locked one survives.
- Two clients, both offline, both play, both upload → second gets `409` with usable metadata.
- Resume an interrupted download and verify the completed file's hash matches a clean download.
