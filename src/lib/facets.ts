import { PRIMARY_FACETS, type FacetFilters, type FacetKey } from "@/state/libraryView";
import type { Game, LibraryEntry } from "@/types";

/** Where each filter reads its values from. One place, so nothing is filtered on but not offered. */
export const FACET_VALUES: Record<FacetKey, (game: Game) => string[]> = {
  genre: (game) => game.genres,
  developer: (game) => game.developers,
  publisher: (game) => game.publishers,
  theme: (game) => game.themes,
  feature: (game) => game.features,
  perspective: (game) => game.perspectives,
  keyword: (game) => game.keywords,
  platform: (game) => game.platforms,
};

export const FACET_LABELS: Record<FacetKey, string> = {
  genre: "All genres",
  developer: "All developers",
  publisher: "All publishers",
  theme: "All themes",
  feature: "All features",
  perspective: "All perspectives",
  keyword: "All keywords",
  platform: "All platforms",
};

export const FACET_KEYS = Object.keys(FACET_VALUES) as FacetKey[];

/** The ones behind the Advanced search row, in the order they are shown. */
export const ADVANCED_FACETS = FACET_KEYS.filter((key) => !PRIMARY_FACETS.includes(key));

/** Whether a game passes every facet, optionally ignoring one so its own list stays full. */
export function matchesFacets(
  entry: LibraryEntry,
  facets: FacetFilters,
  ignore?: FacetKey,
): boolean {
  return FACET_KEYS.every((key) => {
    const wanted = facets[key];
    return key === ignore || !wanted || FACET_VALUES[key](entry.game).includes(wanted);
  });
}

/** The score to filter on: the players' if there is one, else the critics'. */
export function ratingOf(game: Game): number | null {
  return game.userRating ?? game.criticRating ?? null;
}
