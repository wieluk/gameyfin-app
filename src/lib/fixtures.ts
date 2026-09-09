/** Fixtures for browser-only development. Not bundled into the packaged app path. */

import type { Library, LibraryEntry } from "@/types";

export const mockLibraries: Library[] = [
  { id: 1, name: "PC Games", gameIds: [1, 2, 3, 4, 5, 6] },
  { id: 2, name: "Retro", gameIds: [7, 8] },
];

function entry(
  id: number,
  title: string,
  opts: Partial<LibraryEntry> & { genres?: string[]; state?: LibraryEntry["state"] } = {},
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
      genres: opts.genres ?? ["Action"],
      developers: [],
      publishers: [],
      themes: [],
      features: [],
      keywords: [],
      perspectives: [],
      cover: null,
      header: null,
      metadata: { fileSize: 1024 * 1024 * 1024 * (1 + (id % 7)) },
    },
    state: opts.state ?? { kind: "not-installed" },
    archivePresent: false,
    coverUrl: null,
    headerUrl: null,
    screenshotUrls: [],
    minutesPlayed: opts.minutesPlayed ?? 0,
    lastPlayedAt: opts.lastPlayedAt ?? null,
  };
}

export const mockEntries: LibraryEntry[] = [
  entry(1, "Hollow Knight", {
    genres: ["Metroidvania"],
    state: { kind: "installed", path: "/games/hollow-knight", executable: "hollow_knight.exe" },
    minutesPlayed: 1247,
  }),
  entry(2, "Celeste", {
    genres: ["Platformer"],
    state: { kind: "running", since: new Date().toISOString() },
    minutesPlayed: 612,
  }),
  entry(3, "Disco Elysium", {
    genres: ["RPG"],
    state: {
      kind: "downloading",
      receivedBytes: 4.2 * 1024 ** 3,
      totalBytes: 21 * 1024 ** 3,
      bytesPerSecond: 18 * 1024 ** 2,
    },
  }),
  entry(4, "Outer Wilds", { genres: ["Adventure"], state: { kind: "installing", percent: 63 } }),
  entry(5, "Hades", { genres: ["Roguelike"], minutesPlayed: 88 }),
  entry(6, "Return of the Obra Dinn", { genres: ["Puzzle"] }),
  entry(7, "Chrono Trigger", { genres: ["JRPG"] }),
  entry(8, "Super Metroid", { genres: ["Metroidvania"] }),
];
