/** Fully-populated fixtures, so a test can name only the field it is about. */

import type { Game, GameState, LibraryEntry } from "@/types";

export function testGame(overrides: Partial<Game> = {}): Game {
  return {
    id: 1,
    title: "Celeste",
    libraryId: 1,
    summary: null,
    comment: null,
    release: null,
    userRating: null,
    criticRating: null,
    platforms: [],
    genres: [],
    developers: [],
    publishers: [],
    themes: [],
    features: [],
    keywords: [],
    perspectives: [],
    collectionIds: [],
    images: [],
    videoUrls: [],
    cover: null,
    header: null,
    metadata: { fileSize: 0, originalIds: null },
    ...overrides,
  };
}

export function testEntry(state: GameState, overrides: Partial<LibraryEntry> = {}): LibraryEntry {
  return {
    game: testGame(),
    state,
    minutesPlayed: 0,
    lastPlayedAt: null,
    archivePresent: false,
    coverUrl: null,
    headerUrl: null,
    screenshotUrls: [],
    videoUrls: [],
    ...overrides,
  };
}

export function installed(overrides: Partial<Extract<GameState, { kind: "installed" }>> = {}): GameState {
  return {
    kind: "installed",
    path: "/games/celeste",
    executable: null,
    setupCandidates: [],
    stagingSetups: [],
    stagingPresent: false,
    busy: null,
    ...overrides,
  };
}

export function running(overrides: Partial<Extract<GameState, { kind: "running" }>> = {}): GameState {
  return {
    kind: "running",
    since: "2026-01-01T00:00:00Z",
    executable: null,
    runtime: null,
    ...overrides,
  };
}

export function failed(overrides: Partial<Extract<GameState, { kind: "failed" }>> = {}): GameState {
  return {
    kind: "failed",
    message: "boom",
    stage: "download",
    path: null,
    executable: null,
    elevationRequired: false,
    ...overrides,
  };
}
