import { describe, expect, it } from "vitest";

import { SORT_KEYS, directionLabel, sortEntries } from "./sort";
import { testEntry, testGame } from "./testing";
import type { Game, LibraryEntry } from "@/types";

function game(title: string, overrides: Partial<Game> = {}): LibraryEntry {
  return testEntry({ kind: "not-installed" }, { game: testGame({ title, ...overrides }) });
}

const titles = (entries: LibraryEntry[]) => entries.map((e) => e.game.title);

describe("sortEntries", () => {
  const games = [
    game("Hades", { createdAt: "2025-02-01T00:00:00Z", release: "2020-09-17", userRating: 90 }),
    game("Celeste", { createdAt: "2025-03-01T00:00:00Z", release: "2018-01-25", userRating: 80 }),
    game("Tunic", { createdAt: null, release: null, userRating: null }),
    game("Braid", { createdAt: "2025-01-01T00:00:00Z", release: "2008-08-06", userRating: 80 }),
  ];

  it("puts the newest first, and flips to the oldest", () => {
    expect(titles(sortEntries(games, "added", "asc"))).toEqual(["Celeste", "Hades", "Braid", "Tunic"]);
    expect(titles(sortEntries(games, "added", "desc"))).toEqual(["Braid", "Hades", "Celeste", "Tunic"]);
  });

  it("keeps games without a value last in both directions", () => {
    expect(titles(sortEntries(games, "release", "asc")).at(-1)).toBe("Tunic");
    expect(titles(sortEntries(games, "release", "desc")).at(-1)).toBe("Tunic");
  });

  it("breaks ties by title", () => {
    expect(titles(sortEntries(games, "rating", "asc"))).toEqual(["Hades", "Braid", "Celeste", "Tunic"]);
  });

  it("orders titles A to Z, then Z to A", () => {
    expect(titles(sortEntries(games, "title", "asc"))).toEqual(["Braid", "Celeste", "Hades", "Tunic"]);
    expect(titles(sortEntries(games, "title", "desc"))).toEqual(["Tunic", "Hades", "Celeste", "Braid"]);
  });
});

describe("directionLabel", () => {
  it("names both directions of every field", () => {
    for (const key of SORT_KEYS) {
      expect(directionLabel(key, "asc")).not.toBe(directionLabel(key, "desc"));
    }
    expect(directionLabel("added", "asc")).toBe("Newest first");
    expect(directionLabel("rating", "desc")).toBe("Lowest first");
  });
});
