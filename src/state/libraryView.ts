import { create } from "zustand";

/**
 * How the library is currently being viewed.
 *
 * Kept in a store rather than component state so it survives navigating to another tab
 * and back, losing a search and sort every time you check a download is needlessly
 * annoying, and persisted so it also survives a restart.
 */

export type SortKey = "title" | "recent" | "size" | "playtime";
export type SortDirection = "asc" | "desc";
/** Which games to show, by whether they are on this machine. */
export type PresenceFilter = "all" | "installed" | "not-installed";

interface LibraryView {
  search: string;
  sort: SortKey;
  direction: SortDirection;
  libraryId: number | null;
  presence: PresenceFilter;
  setSearch: (search: string) => void;
  setPresence: (presence: PresenceFilter) => void;
  setSort: (sort: SortKey) => void;
  setDirection: (direction: SortDirection) => void;
  toggleDirection: () => void;
  setLibraryId: (libraryId: number | null) => void;
}

const STORAGE_KEY = "gameyfin.library-view";

interface Persisted {
  sort: SortKey;
  direction: SortDirection;
  libraryId: number | null;
  presence: PresenceFilter;
}

function load(): Persisted {
  const fallback: Persisted = {
    sort: "title",
    direction: "asc",
    libraryId: null,
    presence: "all",
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

  setSearch: (search) => set({ search }),
  setPresence: (presence) => {
    set({ presence });
    const { sort, direction, libraryId } = get();
    save({ sort, direction, libraryId, presence });
  },
  setSort: (sort) => {
    set({ sort });
    const { direction, libraryId, presence } = get();
    save({ sort, direction, libraryId, presence });
  },
  setDirection: (direction) => {
    set({ direction });
    const { sort, libraryId, presence } = get();
    save({ sort, direction, libraryId, presence });
  },
  toggleDirection: () => {
    const direction = get().direction === "asc" ? "desc" : "asc";
    get().setDirection(direction);
  },
  setLibraryId: (libraryId) => {
    set({ libraryId });
    const { sort, direction, presence } = get();
    save({ sort, direction, libraryId, presence });
  },
}));
