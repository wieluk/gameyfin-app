import { readStored, writeStored } from "@/lib/storage";
import { create } from "zustand";

/** How the library is being viewed. In a persisted store so it survives tab changes and restarts. */

export type SortKey = "title" | "recent" | "size" | "playtime";
export type SortDirection = "asc" | "desc";
/** How large the covers in the grid are. */
export type CardSize = "small" | "medium" | "large";
export const CARD_SIZES: CardSize[] = ["small", "medium", "large"];

/** A filter on one of the game's list-valued fields. Null means "any". */
export interface FacetFilters {
  genre: string | null;
  developer: string | null;
  publisher: string | null;
  theme: string | null;
  feature: string | null;
  perspective: string | null;
  keyword: string | null;
  platform: string | null;
}

export type FacetKey = keyof FacetFilters;

export const NO_FACETS: FacetFilters = {
  genre: null,
  developer: null,
  publisher: null,
  theme: null,
  feature: null,
  perspective: null,
  keyword: null,
  platform: null,
};

interface LibraryView {
  search: string;
  /** Whether the second row of filters is open. */
  advanced: boolean;
  /** Lowest score to show, out of 100. Null means any, including unrated. */
  minRating: number | null;
  sort: SortKey;
  direction: SortDirection;
  libraryId: number | null;
  installedOnly: boolean;
  cardSize: CardSize;
  facets: FacetFilters;
  setSearch: (search: string) => void;
  setInstalledOnly: (installedOnly: boolean) => void;
  setCardSize: (size: CardSize) => void;
  setSort: (sort: SortKey) => void;
  setDirection: (direction: SortDirection) => void;
  toggleDirection: () => void;
  setLibraryId: (libraryId: number | null) => void;
  setFacet: (facet: FacetKey, value: string | null) => void;
  /** Every filter back to showing the whole library. Sorting is left as it is. */
  resetFilters: () => void;
  toggleAdvanced: () => void;
  setAdvanced: (advanced: boolean) => void;
  setMinRating: (rating: number | null) => void;
}

const STORAGE_KEY = "gameyfin.library-view";

interface Persisted {
  sort: SortKey;
  direction: SortDirection;
  libraryId: number | null;
  installedOnly: boolean;
  cardSize: CardSize;
  facets: FacetFilters;
  advanced: boolean;
  minRating: number | null;
}

function load(): Persisted {
  const fallback: Persisted = {
    sort: "title",
    direction: "asc",
    libraryId: null,
    installedOnly: false,
    cardSize: "medium",
    facets: NO_FACETS,
    advanced: false,
    minRating: null,
  };
  // The legacy `presence` filter: only its "installed" value maps onto the checkbox.
  const parsed = readStored<Partial<Persisted> & { presence?: string }>(STORAGE_KEY, {});
  return {
    sort: parsed.sort ?? fallback.sort,
    direction: parsed.direction ?? fallback.direction,
    libraryId: parsed.libraryId ?? fallback.libraryId,
    installedOnly: parsed.installedOnly ?? parsed.presence === "installed",
    cardSize: parsed.cardSize ?? fallback.cardSize,
    facets: { ...fallback.facets, ...(parsed.facets ?? {}) },
    advanced: parsed.advanced ?? fallback.advanced,
    minRating: parsed.minRating ?? fallback.minRating,
  };
}

function save(state: Persisted) {
  writeStored(STORAGE_KEY, state);
}

const initial = load();

export const useLibraryView = create<LibraryView>((set, get) => ({
  // The search box is deliberately not persisted: coming back to a filtered library
  // after a restart, with no obvious reason why most games are missing, is confusing.
  search: "",
  sort: initial.sort,
  direction: initial.direction,
  libraryId: initial.libraryId,
  installedOnly: initial.installedOnly,
  cardSize: initial.cardSize,
  advanced: initial.advanced,
  minRating: initial.minRating,

  facets: initial.facets,

  setSearch: (search) => set({ search }),
  setInstalledOnly: (installedOnly) => set(persisting({ installedOnly }, get)),
  setCardSize: (cardSize) => set(persisting({ cardSize }, get)),
  setSort: (sort) => set(persisting({ sort }, get)),
  setDirection: (direction) => set(persisting({ direction }, get)),
  toggleDirection: () => {
    const direction = get().direction === "asc" ? "desc" : "asc";
    get().setDirection(direction);
  },
  setLibraryId: (libraryId) => set(persisting({ libraryId }, get)),
  setFacet: (facet, value) =>
    set(persisting({ facets: { ...get().facets, [facet]: value } }, get)),
  resetFilters: () =>
    set({
      search: "",
      ...persisting(
        { libraryId: null, installedOnly: false, facets: NO_FACETS, minRating: null },
        get,
      ),
    }),
  setMinRating: (minRating) => set(persisting({ minRating }, get)),
  toggleAdvanced: () => get().setAdvanced(!get().advanced),
  setAdvanced: (advanced) => set(persisting({ advanced }, get)),
}));

/** Apply a change and persist the whole view, so a new field cannot be left unsaved. */
function persisting(
  change: Partial<Persisted>,
  get: () => LibraryView,
): Partial<LibraryView> {
  const { sort, direction, libraryId, installedOnly, cardSize, facets, advanced, minRating } = {
    ...get(),
    ...change,
  };
  save({
    sort,
    direction,
    libraryId,
    installedOnly,
    cardSize,
    facets,
    advanced,
    minRating,
  });
  return change;
}
