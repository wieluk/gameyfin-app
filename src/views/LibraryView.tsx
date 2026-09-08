import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { GameCard } from "@/components/GameCard";
import { GameDetail } from "@/components/GameDetail";
import { InstallDialog } from "@/components/InstallDialog";
import { Icon } from "@/components/Icon";
import { isInstalled, needsChooser, primaryAction } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { RootChooser, useLibraryRoots } from "@/components/RootChooser";
import {
  useLibraryView,
  type FacetKey,
  type PresenceFilter,
  type SortDirection,
  type SortKey,
} from "@/state/libraryView";
import type { LibraryEntry } from "@/types";
import { useEntries } from "@/lib/queries";
import { useRescanOnOpen } from "@/lib/rescan";


export function LibraryView() {
  // In a store so the view survives switching tabs and restarting.
  const {
    search,
    sort,
    direction,
    libraryId,
    presence,
    setSearch,
    setSort,
    toggleDirection,
    setLibraryId,
    setPresence,
    facets,
    setFacet,
    clearFacets,
  } = useLibraryView();
  const [selected, setSelected] = useState<LibraryEntry | null>(null);
  const [installing, setInstalling] = useState<LibraryEntry | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  // Set when a download needs a destination chosen; null the rest of the time.
  const [choosingRoot, setChoosingRoot] = useState<LibraryEntry | null>(null);
  const roots = useLibraryRoots();

  // Same as Downloads and Installed: a game installed or removed outside the app should
  // show its real state here too, not the one recorded whenever the app last looked.
  useRescanOnOpen();

  const entries = useEntries();
  const libraries = useQuery({ queryKey: ["libraries"], queryFn: () => backend.listLibraries() });

  async function handlePrimaryAction(entry: LibraryEntry) {
    // Extracting and installing involve a choice; downloading and playing do not.
    if (needsChooser(entry.state)) {
      setInstalling(entry);
      return;
    }
    const action = primaryAction(entry.state);
    if (action.disabled) return;

    // Starting a download is the one action that needs to know *where*, and only when
    // there is more than one games folder to choose between.
    if (entry.state.kind === "not-installed" && (roots.data?.length ?? 0) > 1) {
      setChoosingRoot(entry);
      return;
    }

    setActionError(null);
    try {
      await action.run(entry.game.id);
    } catch (e) {
      setActionError(messageOf(e));
    }
  }

  async function downloadTo(entry: LibraryEntry, root: string) {
    setChoosingRoot(null);
    setActionError(null);
    try {
      await backend.startDownload(entry.game.id, root);
    } catch (e) {
      setActionError(messageOf(e));
    }
  }

  const visible = useMemo(() => {
    const all = entries.data ?? [];
    const needle = search.trim().toLowerCase();
    const filtered = all.filter((e) => {
      if (libraryId !== null && e.game.libraryId !== libraryId) return false;
      if (presence !== "all" && (presence === "installed") !== isInstalled(e)) return false;
      if (facets.genre && !e.game.genres.includes(facets.genre)) return false;
      if (facets.developer && !e.game.developers.includes(facets.developer)) return false;
      if (facets.publisher && !e.game.publishers.includes(facets.publisher)) return false;
      if (!needle) return true;
      return (
        e.game.title.toLowerCase().includes(needle) ||
        e.game.genres.some((g) => g.toLowerCase().includes(needle))
      );
    });
    return sortEntries(filtered, sort, direction);
  }, [entries.data, search, sort, direction, libraryId, presence, facets]);

  // Built from what is actually in the library rather than from a fixed list, so a filter
  // never offers a value that would match nothing. Narrowed by the other filters for the
  // same reason: after picking a developer, only their genres are worth offering.
  const options = useMemo(() => {
    const all = entries.data ?? [];
    const inScope = all.filter(
      (e) =>
        (libraryId === null || e.game.libraryId === libraryId) &&
        (presence === "all" || (presence === "installed") === isInstalled(e)),
    );
    const collect = (pick: (e: LibraryEntry) => string[], ignore: FacetKey) => {
      const matching = inScope.filter(
        (e) =>
          (ignore === "genre" || !facets.genre || e.game.genres.includes(facets.genre)) &&
          (ignore === "developer" ||
            !facets.developer ||
            e.game.developers.includes(facets.developer)) &&
          (ignore === "publisher" ||
            !facets.publisher ||
            e.game.publishers.includes(facets.publisher)),
      );
      return [...new Set(matching.flatMap(pick))].sort((a, b) => a.localeCompare(b));
    };
    return {
      genre: collect((e) => e.game.genres, "genre"),
      developer: collect((e) => e.game.developers, "developer"),
      publisher: collect((e) => e.game.publishers, "publisher"),
    };
  }, [entries.data, libraryId, presence, facets]);

  const activeFacets = Object.values(facets).filter(Boolean).length;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center gap-3 border-b border-default-200/60 px-6 py-3">
        <div className="relative flex-1 max-w-md">
          <Icon name="search" className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-foreground/40" />
          <input
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search your library"
            className="w-full rounded-lg border border-default-200 bg-content2 py-2 pl-9 pr-3 text-sm outline-none transition-colors placeholder:text-foreground/40 focus:border-primary"
          />
        </div>

        <select
          value={libraryId ?? ""}
          onChange={(e) => setLibraryId(e.target.value === "" ? null : Number(e.target.value))}
          className="rounded-lg border border-default-200 bg-content2 px-3 py-2 text-sm outline-none focus:border-primary"
        >
          <option value="">All libraries</option>
          {(libraries.data ?? []).map((l) => (
            <option key={l.id} value={l.id}>{l.name}</option>
          ))}
        </select>

        <select
          value={presence}
          onChange={(e) => setPresence(e.target.value as PresenceFilter)}
          className="rounded-lg border border-default-200 bg-content2 px-3 py-2 text-sm outline-none focus:border-primary"
        >
          <option value="all">All games</option>
          <option value="installed">Installed</option>
          <option value="not-installed">Not installed</option>
        </select>

        <Facet
          label="All genres"
          value={facets.genre}
          options={options.genre}
          onChange={(value) => setFacet("genre", value)}
        />
        <Facet
          label="All developers"
          value={facets.developer}
          options={options.developer}
          onChange={(value) => setFacet("developer", value)}
        />
        <Facet
          label="All publishers"
          value={facets.publisher}
          options={options.publisher}
          onChange={(value) => setFacet("publisher", value)}
        />

        {activeFacets > 0 && (
          <button
            type="button"
            onClick={clearFacets}
            title="Clear the genre, developer and publisher filters"
            className="shrink-0 rounded-lg border border-default-200 bg-content2 px-2.5 py-2 text-xs text-foreground/70 transition-colors hover:bg-default-100"
          >
            Clear
          </button>
        )}

        <select
          value={sort}
          onChange={(e) => setSort(e.target.value as SortKey)}
          className="rounded-lg border border-default-200 bg-content2 px-3 py-2 text-sm outline-none focus:border-primary"
        >
          <option value="title">Title</option>
          <option value="recent">Last played</option>
          <option value="playtime">Playtime</option>
          <option value="size">Size</option>
        </select>

        <button
          type="button"
          onClick={toggleDirection}
          title={directionLabel(sort, direction)}
          aria-label={directionLabel(sort, direction)}
          className="flex items-center gap-1 rounded-lg border border-default-200 bg-content2 px-2.5 py-2 text-xs text-foreground/70 transition-colors hover:bg-default-100"
        >
          <Icon
            name="chevron"
            className={`h-4 w-4 transition-transform ${
              direction === "asc" ? "-rotate-90" : "rotate-90"
            }`}
          />
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
        {entries.isLoading ? (
          <SkeletonGrid />
        ) : visible.length === 0 ? (
          <EmptyState search={search} />
        ) : (
          <div data-library-grid className="grid grid-cols-[repeat(auto-fill,minmax(160px,1fr))] gap-5">
            {visible.map((entry) => (
              <GameCard
                key={entry.game.id}
                entry={entry}
                onPrimaryAction={(e) => void handlePrimaryAction(e)}
                onOpen={setSelected}
              />
            ))}
          </div>
        )}
      </div>

      {actionError && (
        <div className="pointer-events-none fixed inset-x-0 bottom-4 flex justify-center px-6">
          <p
            role="alert"
            className="pointer-events-auto max-w-xl rounded-lg border border-danger/30 bg-danger/95 px-4 py-2 text-xs text-white shadow-lg"
            onClick={() => setActionError(null)}
          >
            {actionError}
          </p>
        </div>
      )}

      {installing && (
        <InstallDialog entry={installing} onClose={() => setInstalling(null)} />
      )}

      {choosingRoot && (
        <RootChooser
          title={choosingRoot.game.title}
          requiredBytes={choosingRoot.game.metadata.fileSize || undefined}
          onChoose={(root) => void downloadTo(choosingRoot, root)}
          onCancel={() => setChoosingRoot(null)}
        />
      )}

      {selected && (
        <GameDetail
          // Re-read from the query so progress keeps updating while the dialog is open.
          entry={visible.find((e) => e.game.id === selected.game.id) ?? selected}
          onClose={() => setSelected(null)}
          onPrimaryAction={handlePrimaryAction}
        />
      )}
    </div>
  );
}



/**
 * One of the value filters.
 *
 * Hidden when there is nothing to choose from. A dropdown whose only entry is "All" is a
 * control that cannot do anything, and the header has enough in it already.
 */
function Facet({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string | null;
  options: string[];
  onChange: (value: string | null) => void;
}) {
  // Kept when it is the current selection even if nothing matches any more, so a filter
  // can always be undone from the control that set it.
  if (options.length === 0 && !value) return null;

  return (
    <select
      value={value ?? ""}
      onChange={(e) => onChange(e.target.value === "" ? null : e.target.value)}
      aria-label={label}
      className={`max-w-[10rem] shrink-0 rounded-lg border bg-content2 px-3 py-2 text-sm outline-none focus:border-primary ${
        value ? "border-primary/50 text-primary" : "border-default-200"
      }`}
    >
      <option value="">{label}</option>
      {value && !options.includes(value) && <option value={value}>{value}</option>}
      {options.map((option) => (
        <option key={option} value={option}>
          {option}
        </option>
      ))}
    </select>
  );
}

function sortEntries(
  entries: LibraryEntry[],
  sort: SortKey,
  direction: SortDirection,
): LibraryEntry[] {
  // Compare in a fixed "natural" order, then reverse, so the toggle means the same thing
  // for every field rather than each having its own idea of which way is up.
  const compare = (a: LibraryEntry, b: LibraryEntry): number => {
    switch (sort) {
      case "recent":
        return (b.lastPlayedAt ?? "").localeCompare(a.lastPlayedAt ?? "");
      case "playtime":
        return b.minutesPlayed - a.minutesPlayed;
      case "size":
        return b.game.metadata.fileSize - a.game.metadata.fileSize;
      default:
        return a.game.title.localeCompare(b.game.title);
    }
  };

  const sorted = [...entries].sort(compare);
  return direction === "asc" ? sorted : sorted.reverse();
}

/** Spell out what the current direction actually means for this field. */
function directionLabel(sort: SortKey, direction: SortDirection): string {
  const ascending = direction === "asc";
  switch (sort) {
    case "size":
      return ascending ? "Largest first" : "Smallest first";
    case "playtime":
      return ascending ? "Most played first" : "Least played first";
    case "recent":
      return ascending ? "Most recent first" : "Oldest first";
    default:
      return ascending ? "A to Z" : "Z to A";
  }
}

function SkeletonGrid() {
  return (
    <div data-library-grid className="grid grid-cols-[repeat(auto-fill,minmax(160px,1fr))] gap-5">
      {Array.from({ length: 12 }).map((_, i) => (
        <div key={i} className="flex flex-col gap-2">
          <div className="aspect-[2/3] animate-pulse rounded-xl bg-default-200" />
          <div className="h-3 w-3/4 animate-pulse rounded bg-default-200" />
        </div>
      ))}
    </div>
  );
}

function EmptyState({ search }: { search: string }) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 text-center">
      <Icon name="library" className="h-10 w-10 text-foreground/25" />
      <p className="text-sm text-foreground/60">
        {search ? `Nothing matches "${search}"` : "This library is empty"}
      </p>
    </div>
  );
}
