import { describe, expect, it } from "vitest";

import { FACET_KEYS, FACET_LABELS, FACET_VALUES, matchesFacets, ratingOf } from "./facets";
import { NO_FACETS } from "@/state/libraryView";
import { testEntry, testGame } from "./testing";
import type { Game, LibraryEntry } from "@/types";

function game(overrides: Partial<Game>): LibraryEntry {
  return testEntry({ kind: "not-installed" }, { game: testGame(overrides) });
}

describe("matchesFacets", () => {
  it("passes everything when nothing is chosen", () => {
    expect(matchesFacets(game({}), NO_FACETS)).toBe(true);
  });

  it("narrows on a field the main bar does not offer", () => {
    const platformer = game({ themes: ["Comedy"] });
    expect(matchesFacets(platformer, { ...NO_FACETS, theme: "Comedy" })).toBe(true);
    expect(matchesFacets(platformer, { ...NO_FACETS, theme: "Horror" })).toBe(false);
  });

  it("requires every chosen filter, not just one", () => {
    const both = game({ themes: ["Comedy"], keywords: ["pixel art"] });
    expect(
      matchesFacets(both, { ...NO_FACETS, theme: "Comedy", keyword: "pixel art" }),
    ).toBe(true);
    expect(matchesFacets(both, { ...NO_FACETS, theme: "Comedy", keyword: "roguelike" })).toBe(
      false,
    );
  });

  it("ignores the one filter it is building options for", () => {
    // Otherwise choosing a theme empties the theme list and the choice cannot be changed.
    const only = game({ themes: ["Comedy"] });
    expect(matchesFacets(only, { ...NO_FACETS, theme: "Horror" }, "theme")).toBe(true);
  });
});

describe("ratingOf", () => {
  it("prefers the players' score over the critics'", () => {
    expect(ratingOf(game({ userRating: 80, criticRating: 60 }).game)).toBe(80);
  });

  it("falls back to the critics' when there is no player score", () => {
    expect(ratingOf(game({ userRating: null, criticRating: 60 }).game)).toBe(60);
  });

  it("is null when a game has no score at all", () => {
    expect(ratingOf(game({ userRating: null, criticRating: null }).game)).toBeNull();
  });
});

describe("the facet tables", () => {
  it("label and read every key, so none can be filtered on but not offered", () => {
    for (const key of FACET_KEYS) {
      expect(FACET_LABELS[key], key).toBeTruthy();
      expect(typeof FACET_VALUES[key], key).toBe("function");
    }
    // Every filter the state knows about reaches the Advanced row.
    expect(FACET_KEYS.sort()).toEqual(Object.keys(NO_FACETS).sort());
  });
});
