/** Library ordering. Each field has one natural order, and the direction toggle flips it. */

import { ratingOf } from "@/lib/facets";
import type { SortDirection, SortKey } from "@/state/libraryView";
import type { LibraryEntry } from "@/types";

export const SORT_LABELS: Record<SortKey, string> = {
  title: "Title",
  added: "Date added",
  updated: "Last updated",
  release: "Release date",
  rating: "Rating",
  recent: "Last played",
  playtime: "Playtime",
  size: "Size",
};

export const SORT_KEYS = Object.keys(SORT_LABELS) as SortKey[];

/** What the natural order and its reverse mean for each field. */
const DIRECTION_LABELS: Record<SortKey, [string, string]> = {
  title: ["A to Z", "Z to A"],
  added: ["Newest first", "Oldest first"],
  updated: ["Newest first", "Oldest first"],
  release: ["Newest first", "Oldest first"],
  rating: ["Highest first", "Lowest first"],
  recent: ["Most recent first", "Oldest first"],
  playtime: ["Most played first", "Least played first"],
  size: ["Largest first", "Smallest first"],
};

export function directionLabel(sort: SortKey, direction: SortDirection): string {
  const [natural, reversed] = DIRECTION_LABELS[sort] ?? DIRECTION_LABELS.title;
  return direction === "asc" ? natural : reversed;
}

function time(value: string | null | undefined): number | null {
  const parsed = value ? Date.parse(value) : NaN;
  return Number.isNaN(parsed) ? null : parsed;
}

/** The number a field sorts on, largest first, or null when the game has none. */
function valueOf(entry: LibraryEntry, sort: SortKey): number | null {
  switch (sort) {
    case "added":
      return time(entry.game.createdAt);
    case "updated":
      return time(entry.game.updatedAt);
    case "release":
      return time(entry.game.release);
    case "rating":
      return ratingOf(entry.game);
    case "recent":
      return time(entry.lastPlayedAt);
    case "playtime":
      return entry.minutesPlayed;
    case "size":
      return entry.game.metadata.fileSize;
    default:
      return null;
  }
}

/** Games missing the value go last either way, and ties fall back to the title. */
export function sortEntries(
  entries: LibraryEntry[],
  sort: SortKey,
  direction: SortDirection,
): LibraryEntry[] {
  const sign = direction === "asc" ? 1 : -1;
  const byTitle = (a: LibraryEntry, b: LibraryEntry) => a.game.title.localeCompare(b.game.title);

  if (sort === "title" || !(sort in SORT_LABELS)) {
    return [...entries].sort((a, b) => sign * byTitle(a, b));
  }

  return [...entries].sort((a, b) => {
    const x = valueOf(a, sort);
    const y = valueOf(b, sort);
    if (x === null || y === null) {
      if (x !== y) return x === null ? 1 : -1;
    } else if (x !== y) {
      return sign * (y - x);
    }
    return byTitle(a, b);
  });
}
