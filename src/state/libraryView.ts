import { create } from "zustand";

/** How the library is being viewed. In a persisted store so it survives tab changes and restarts. */

export type SortKey = "title" | "recent" | "size" | "playtime";
export type SortDirection = "asc" | "desc";
/** Which games to show, by whether they are on this machine. */
export type PresenceFilter = "all" | "installed" | "not-installed";

/** A filter on one of the game's list-valued fields. Null means "any". */
export interface FacetFilters {
  genre: string | null;
  developer: string | null;
  publisher: string | null;
}

export type FacetKey = keyof FacetFilters;

interface LibraryView {
  search: string;
  sort: SortKey;
  direction: SortDirection;
  libraryId: number | null;
  presence: PresenceFilter;
  facets: FacetFilters;
  setSearch: (search: string) => void;
  setPresence: (presence: PresenceFilter) => void;
  setSort: (sort: SortKey) => void;
  setDirection: (direction: SortDirection) => void;
  toggleDirection: () => void;
  setLibraryId: (libraryId: number | null) => void;
  setFacet: (facet: FacetKey, value: string | null) => void;
  clearFacets: () => void;
}

const STORAGE_KEY = "gameyfin.library-view";

interface Persisted {
  sort: SortKey;
  direction: SortDirection;
  libraryId: number | null;
  presence: PresenceFilter;
  facets: FacetFilters;
}

function load(): Persisted {
  const fallback: Persisted = {
    sort: "title",
    direction: "asc",
    libraryId: null,
    presence: "all",
    facets: { genre: null, developer: null, publisher: null },
  };
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return fallback;
    const parsed = JSON.parse(raw) as Partial<Persisted>;
    return {
      sort: parsed.sort ?? fallback.sort,
      direction: parsed.direction ?? fallback.direction,
      libraryId: parsed.libraryId ?? fallback.libraryId,
      presence: parsed.presence ?? fallback.presence,
      facets: { ...fallback.facets, ...(parsed.facets ?? {}) },
    };
  } catch {
    // Private windows and blocked site data both throw; the defaults are fine.
    return fallback;
  }
}

function save(state: Persisted) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    // A remembered view is a convenience, not a requirement.
  }
}

const initial = load();

export const useLibraryView = create<LibraryView>((set, get) => ({
  // The search box is deliberately not persisted: coming back to a filtered library
  // after a restart, with no obvious reason why most games are missing, is confusing.
  search: "",
  sort: initial.sort,
  direction: initial.direction,
  libraryId: initial.libraryId,
  presence: initial.presence,

  facets: initial.facets,

  setSearch: (search) => set({ search }),
  setPresence: (presence) => set(persisting({ presence }, get)),
  setSort: (sort) => set(persisting({ sort }, get)),
  setDirection: (direction) => set(persisting({ direction }, get)),
  toggleDirection: () => {
    const direction = get().direction === "asc" ? "desc" : "asc";
    get().setDirection(direction);
  },
  setLibraryId: (libraryId) => set(persisting({ libraryId }, get)),
  setFacet: (facet, value) =>
    set(persisting({ facets: { ...get().facets, [facet]: value } }, get)),
  clearFacets: () =>
    set(persisting({ facets: { genre: null, developer: null, publisher: null } }, get)),
}));

/** Apply a change and persist the whole view, so a new field cannot be left unsaved. */
function persisting(
  change: Partial<Persisted>,
  get: () => LibraryView,
): Partial<LibraryView> {
  const { sort, direction, libraryId, presence, facets } = { ...get(), ...change };
  save({ sort, direction, libraryId, presence, facets });
  return change;
}
