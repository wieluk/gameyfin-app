import { describe, expect, it } from "vitest";
import { isInDownloads, isInstalled, isLocal } from "./actions";
import type { GameState, LibraryEntry } from "@/types";

/** Only `state` matters to these helpers; the rest is filler. */
function entry(state: GameState): LibraryEntry {
  return {
    game: {
      id: 1,
      title: "Celeste",
      libraryId: 1,
      platforms: [],
      genres: [],
      developers: [],
      publishers: [],
      metadata: { fileSize: 0 },
    },
    state,
    minutesPlayed: 0,
  };
}

describe("isLocal", () => {
  it("counts everything the user could act on without the server", () => {
    expect(isLocal(entry({ kind: "installed", path: "/games/celeste" }))).toBe(true);
    expect(isLocal(entry({ kind: "running", since: "2026-01-01T00:00:00Z" }))).toBe(true);
    expect(isLocal(entry({ kind: "downloaded", archivePath: "/a.zip", bytes: 10 }))).toBe(true);
    expect(
      isLocal(entry({ kind: "extracted", path: "/x", setupCandidates: [], archivePresent: true })),
    ).toBe(true);
  });

  it("excludes a game that exists only on the server", () => {
    expect(isLocal(entry({ kind: "not-installed" }))).toBe(false);
  });

  it("excludes transfers with nothing usable on disk yet", () => {
    // These appear in Downloads, but offering them in an offline library would promise
    // something the user cannot finish without the server.
    expect(
      isLocal(entry({ kind: "downloading", receivedBytes: 1, totalBytes: 9, bytesPerSecond: 1 })),
    ).toBe(false);
    expect(isLocal(entry({ kind: "preparing", message: "Setting up" }))).toBe(false);
    expect(isLocal(entry({ kind: "failed", message: "no", stage: "download" }))).toBe(false);
  });

  it("is narrower than the Downloads filter", () => {
    const failed = entry({ kind: "failed", message: "no", stage: "download" });
    expect(isInDownloads(failed)).toBe(true);
    expect(isLocal(failed)).toBe(false);
  });

  it("agrees with isInstalled for installed games", () => {
    const installed = entry({ kind: "installed", path: "/games/celeste" });
    expect(isInstalled(installed)).toBe(true);
    expect(isLocal(installed)).toBe(true);
  });
});
