import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { GameCard } from "@/components/GameCard";
import { GameDetail } from "@/components/GameDetail";
import { InstallDialog } from "@/components/InstallDialog";
import { Icon } from "@/components/Icon";
import { isInstalled, isLocal, needsChooser, primaryAction } from "@/lib/actions";
import {
  FACET_KEYS,
  FACET_LABELS,
  FACET_VALUES,
  matchesFacets,
  ratingOf,
} from "@/lib/facets";
import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { RootChooser, useLibraryRoots } from "@/components/RootChooser";
import {
  CARD_SIZES,
  useLibraryView,
  type CardSize,
  type FacetKey,
  type SortDirection,
  type SortKey,
} from "@/state/libraryView";
import type { LibraryEntry } from "@/types";
import { useEntries, useStatus } from "@/lib/queries";
import { useRescanOnOpen } from "@/lib/rescan";
import { INPUT, PANEL_BODY } from "@/lib/ui";


export function LibraryView() {
  // In a store so the view survives switching tabs and restarting.
  const {
    search,
    sort,
    direction,
    libraryId,
    installedOnly,
    cardSize,
    setSearch,
    setSort,
    toggleDirection,
    setLibraryId,
    setInstalledOnly,
    setCardSize,
    advanced,
    toggleAdvanced,
    minRating,
    setMinRating,
    facets,
    setFacet,
    resetFilters,
  } = useLibraryView();
  const [selected, setSelected] = useState<LibraryEntry | null>(null);
  const [installing, setInstalling] = useState<LibraryEntry | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  // Set when a download needs a destination chosen; null the rest of the time.
  const [choosingRoot, setChoosingRoot] = useState<LibraryEntry | null>(null);
  const roots = useLibraryRoots();

  // As in Downloads and Installed: reflect changes made outside the app.
  useRescanOnOpen();

  const entries = useEntries();
  const offline = Boolean(useStatus().data?.offline);
  const libraries = useQuery({ queryKey: ["libraries"], queryFn: () => backend.listLibraries() });

  async function handlePrimaryAction(entry: LibraryEntry) {
    // Extracting and installing involve a choice; downloading and playing do not.
    if (needsChooser(entry.state)) {
      setInstalling(entry);
      return;
    }
    const action = primaryAction(entry.state);
    if (action.disabled) return;

    // Starting a download needs a destination, but only when there is more than one folder.
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
      // The catalogue is served from the offline mirror while the server is unreachable,
      // so without this the list offers games that cannot be downloaded right now.
      if (offline && !isLocal(e)) return false;
      if (libraryId !== null && e.game.libraryId !== libraryId) return false;
      if (installedOnly && !isInstalled(e)) return false;
      if (!matchesFacets(e, facets)) return false;
      if (minRating !== null && (ratingOf(e.game) ?? -1) < minRating) return false;
      if (!needle) return true;
      return (
        e.game.title.toLowerCase().includes(needle) ||
        e.game.genres.some((g) => g.toLowerCase().includes(needle))
      );
    });
    return sortEntries(filtered, sort, direction);
  }, [
    entries.data,
    search,
    sort,
    direction,
    libraryId,
    installedOnly,
    facets,
    minRating,
    offline,
  ]);

  // Built from the current library, and narrowed by the other filters, so no option ever matches nothing.
  const options = useMemo(() => {
    const all = entries.data ?? [];
    const inScope = all.filter(
      (e) =>
        (!offline || isLocal(e)) &&
        (libraryId === null || e.game.libraryId === libraryId) &&
        (!installedOnly || isInstalled(e)),
    );
    // Each list ignores its own filter, so choosing a value never empties the box it
    // came from, and respects the others, so no option is offered that matches nothing.
    const collect = (key: FacetKey) => {
      const matching = inScope.filter((e) => matchesFacets(e, facets, key));
      return [...new Set(matching.flatMap((e) => FACET_VALUES[key](e.game)))].sort((a, b) =>
        a.localeCompare(b),
      );
    };
    return Object.fromEntries(FACET_KEYS.map((key) => [key, collect(key)])) as Record<
      FacetKey,
      string[]
    >;
  }, [entries.data, libraryId, installedOnly, facets, offline]);

  const hiddenOffline = useMemo(
    () => (offline ? (entries.data ?? []).filter((e) => !isLocal(e)).length : 0),
    [entries.data, offline],
  );

  // Shown on the Advanced button, so a closed row never narrows the list unannounced.
  const activeFacets =
    Object.values(facets).filter(Boolean).length + (minRating === null ? 0 : 1);
  const filtered =
    search.trim() !== "" || libraryId !== null || installedOnly || activeFacets > 0;

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
          className={INPUT}
        >
          <option value="">All libraries</option>
          {(libraries.data ?? []).map((l) => (
            <option key={l.id} value={l.id}>{l.name}</option>
          ))}
        </select>

        <label className="flex shrink-0 cursor-pointer items-center gap-2 text-sm text-foreground/70 transition-colors hover:text-foreground">
          <input
            type="checkbox"
            checked={installedOnly}
            onChange={(e) => setInstalledOnly(e.target.checked)}
            className="h-4 w-4 accent-primary"
          />
          Installed only
        </label>

        <button
          type="button"
          onClick={toggleAdvanced}
          aria-expanded={advanced}
          title="More ways to narrow the list"
          className={`flex shrink-0 items-center gap-1 rounded-lg border px-2.5 py-2 text-xs transition-colors ${
            advanced || activeFacets > 0
              ? "border-primary/50 bg-primary/10 text-primary"
              : "border-default-200 bg-content2 text-foreground/70 hover:bg-default-100"
          }`}
        >
          Advanced
          {activeFacets > 0 && <span className="tabular-nums">({activeFacets})</span>}
        </button>

        {/* Always there, so its place never shifts; greyed out while nothing is narrowed. */}
        <button
          type="button"
          onClick={resetFilters}
          disabled={!filtered}
          title="Reset filters"
          aria-label="Reset filters"
          className="flex shrink-0 items-center justify-center rounded-lg border border-default-200 bg-content2 p-2 text-foreground/70 transition-colors hover:bg-default-100 hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
        >
          <Icon name="reset" className="h-4 w-4" />
        </button>

        <CardSizes value={cardSize} onChange={setCardSize} />

        {/* A pill rather than another boxed select, so ordering does not read as a filter. */}
        <div
          role="group"
          aria-label="Sort"
          className="flex shrink-0 items-center rounded-full border border-default-200 text-xs text-foreground/50"
        >
          <Icon name="sort" className="ml-3 h-3.5 w-3.5 shrink-0" />
          <span className="pl-1.5">Sort</span>
          <select
            value={sort}
            onChange={(e) => setSort(e.target.value as SortKey)}
            aria-label="Sort by"
            className="cursor-pointer appearance-none bg-transparent py-2 pl-1.5 pr-2.5 text-xs font-medium text-foreground outline-none focus-visible:underline"
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
            className="flex items-center self-stretch rounded-r-full border-l border-default-200 pl-2 pr-2.5 text-foreground/70 transition-colors hover:bg-default-100"
          >
            <Icon
              name="chevron"
              className={`h-3.5 w-3.5 transition-transform ${
                direction === "asc" ? "-rotate-90" : "rotate-90"
              }`}
            />
          </button>
        </div>
      </div>

      {advanced && (
        <div className="flex shrink-0 flex-wrap items-center gap-3 border-b border-default-200/60 bg-content1/40 px-6 py-3">
          {FACET_KEYS.map((key) => (
            <Facet
              key={key}
              label={FACET_LABELS[key]}
              value={facets[key]}
              options={options[key]}
              onChange={(value) => setFacet(key, value)}
            />
          ))}

          <select
            value={minRating ?? ""}
            onChange={(e) => setMinRating(e.target.value === "" ? null : Number(e.target.value))}
            className={INPUT}
          >
            <option value="">Any rating</option>
            {[90, 80, 70, 60, 50].map((score) => (
              <option key={score} value={score}>
                {score} or higher
              </option>
            ))}
          </select>
        </div>
      )}

      <div className={PANEL_BODY}>
        {entries.isLoading ? (
          <SkeletonGrid />
        ) : visible.length === 0 ? (
          <EmptyState search={search} hiddenOffline={hiddenOffline} />
        ) : (
          <div data-library-grid data-card-size={cardSize} className="grid gap-5">
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
          // Re-read from every entry, not the filtered list: installing a game from its
          // dialog changes its state, and the "not installed" filter would drop it here.
          entry={(entries.data ?? []).find((e) => e.game.id === selected.game.id) ?? selected}
          onClose={() => setSelected(null)}
          onPrimaryAction={handlePrimaryAction}
        />
      )}
    </div>
  );
}



/** How large the covers are, drawn as three squares: the tiles are art, not text. */
function CardSizes({
  value,
  onChange,
}: {
  value: CardSize;
  onChange: (size: CardSize) => void;
}) {
  const square: Record<CardSize, string> = {
    small: "h-2 w-2",
    medium: "h-3 w-3",
    large: "h-4 w-4",
  };
  const label: Record<CardSize, string> = {
    small: "Small covers",
    medium: "Medium covers",
    large: "Large covers",
  };

  return (
    <div
      role="group"
      aria-label="Cover size"
      className="ml-auto flex shrink-0 items-center gap-0.5 rounded-full border border-default-200 px-1.5 py-1"
    >
      {CARD_SIZES.map((size) => (
        <button
          key={size}
          type="button"
          onClick={() => onChange(size)}
          title={label[size]}
          aria-label={label[size]}
          aria-pressed={value === size}
          className={`flex h-6 w-6 items-center justify-center rounded-full transition-colors ${
            value === size ? "text-primary" : "text-foreground/40 hover:text-foreground"
          }`}
        >
          <span className={`${square[size]} rounded-[3px] border-2 border-current`} />
        </button>
      ))}
    </div>
  );
}

/** One of the value filters. Hidden when there is nothing to choose from. */
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
  // The current selection is kept even if nothing matches, so it can always be undone.
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
  // Fixed natural order then reverse, so the direction toggle means the same for every field.
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
    <div data-library-grid className="grid gap-5">
      {Array.from({ length: 12 }).map((_, i) => (
        <div key={i} className="flex flex-col gap-2">
          <div className="aspect-[2/3] animate-pulse rounded-xl bg-default-200" />
          <div className="h-3 w-3/4 animate-pulse rounded bg-default-200" />
        </div>
      ))}
    </div>
  );
}

function EmptyState({ search, hiddenOffline }: { search: string; hiddenOffline: number }) {
  // An empty library while offline is not an empty library, and saying so stops it looking
  // like the catalogue was lost.
  const offlineNote =
    hiddenOffline > 0
      ? `${hiddenOffline} ${hiddenOffline === 1 ? "game is" : "games are"} on your server, which is unreachable right now.`
      : null;

  return (
    <div className="flex h-full flex-col items-center justify-center gap-2 text-center">
      <Icon name={hiddenOffline > 0 ? "offline" : "library"} className="h-10 w-10 text-foreground/25" />
      <p className="text-sm text-foreground/60">
        {search
          ? `Nothing matches "${search}"`
          : hiddenOffline > 0
            ? "Nothing is installed on this PC yet"
            : "This library is empty"}
      </p>
      {offlineNote && !search && <p className="text-xs text-foreground/40">{offlineNote}</p>}
    </div>
  );
}
