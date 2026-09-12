/**
 * Answers for `vite dev` in a plain browser, where no Tauri command exists. Loaded only
 * when there is no Tauri, so none of this reaches the packaged app.
 */

import type { LibraryEntry } from "@/bindings/LibraryEntry";
import type { PublicSettings } from "@/bindings/PublicSettings";

function entry(
  id: number,
  title: string,
  genre: string,
  state: LibraryEntry["state"] = { kind: "not-installed" },
  minutesPlayed = 0,
): LibraryEntry {
  return {
    game: {
      id,
      title,
      libraryId: id <= 6 ? 1 : 2,
      summary: null,
      release: null,
      userRating: null,
      criticRating: null,
      platforms: ["Windows"],
      genres: [genre],
      developers: [],
      publishers: [],
      themes: [],
      features: [],
      keywords: [],
      perspectives: [],
      comment: null,
      collectionIds: [],
      images: [],
      videoUrls: [],
      cover: null,
      header: null,
      metadata: { fileSize: 1024 ** 3 * (1 + (id % 7)), originalIds: null },
    },
    state,
    archivePresent: false,
    coverUrl: null,
    headerUrl: null,
    screenshotUrls: [],
    videoUrls: [],
    minutesPlayed,
    lastPlayedAt: null,
  };
}

const entries: LibraryEntry[] = [
  entry(1, "Hollow Knight", "Metroidvania", {
    kind: "installed",
    path: "/games/hollow-knight",
    executable: "hollow_knight.exe",
    setupCandidates: [],
    stagingSetups: [],
    stagingPresent: false,
    busy: null,
  }, 1247),
  entry(2, "Celeste", "Platformer", {
    kind: "running",
    since: new Date().toISOString(),
    executable: "Celeste.exe",
    runtime: "UMU-Proton",
  }, 612),
  entry(3, "Disco Elysium", "RPG", {
    kind: "downloading",
    receivedBytes: 4.2 * 1024 ** 3,
    totalBytes: 21 * 1024 ** 3,
    bytesPerSecond: 18 * 1024 ** 2,
  }),
  entry(4, "Outer Wilds", "Adventure", { kind: "installing" }),
  entry(5, "Hades", "Roguelike", { kind: "not-installed" }, 88),
  entry(6, "Return of the Obra Dinn", "Puzzle"),
  entry(7, "Chrono Trigger", "JRPG"),
  entry(8, "Super Metroid", "Metroidvania"),
];

const settings: Partial<PublicSettings> = {
  logLevel: "info",
  libraryRoot: "/games",
  theme: "dark",
  installerMemoryLimit: "auto",
  saveBackend: "server",
  saveSyncEnabled: true,
};

/** Everything not listed answers with `null`, which reads as "nothing configured". */
const answers: Record<string, unknown> = {
  list_libraries: [
    { id: 1, name: "PC Games", gameIds: [1, 2, 3, 4, 5, 6] },
    { id: 2, name: "Retro", gameIds: [7, 8] },
  ],
  list_entries: entries,
  // Signed in, since a browser has no window to sign in with.
  connection_status: {
    configured: true,
    authenticated: true,
    offline: false,
    serverUrl: "https://demo.invalid",
    username: "demo",
  },
  get_settings: settings,
  list_library_roots: [{ path: "/games", isDefault: true, freeBytes: 512 * 1024 ** 3, exists: true }],
  download_providers: [],
  list_executables: [],
  list_save_versions: [],
  save_overview: [],
  scan_this_pc: [],
  skip_save_sync: null,
  save_locations: { savesRoot: null, staging: null, prefixHome: null, prefixDriveC: null, installDir: null, home: null, detected: [] },
  list_prefixes: [],
  rescan_library: 0,
  save_state: { kind: "off" },
  update_status: { currentVersion: "0.0.0-dev", available: false, canInstall: false },
};

export async function mockInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  console.info(`[mock] ${command}`, args ?? {});
  return (answers[command] ?? null) as T;
}
