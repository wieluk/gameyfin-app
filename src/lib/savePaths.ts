/** Helpers for the save folders dialog, kept pure so they can be tested. */

import type { SaveLocations } from "@/bindings/SaveLocations";

/** One rewrite: where the save is on this PC, and the name it is stored under. */
export interface Mapping {
  source: string;
  target: string;
}

/** Under the synthetic root, so a restore that misses the correction is reported as misplaced. */
const STORED_ROOT = "/gameyfin/custom/";

/** A stored name built from the title, so every PC arrives at the same one on its own. */
export function storedNameFor(title: string, taken: readonly string[]): string {
  const slug =
    title
      .toLowerCase()
      .replace(/[^\p{L}\p{N}]+/gu, "-")
      .replace(/^-+|-+$/g, "") || "game";
  const base = `${STORED_ROOT}${slug}`;
  if (!taken.includes(base)) return base;
  let n = 2;
  while (taken.includes(`${base}-${n}`)) n++;
  return `${base}-${n}`;
}

/** Fills blank stored names in order, so two new corrections never share one. */
export function fillStoredNames(rows: readonly Mapping[], title: string): Mapping[] {
  const taken = rows.map((row) => row.target.trim()).filter(Boolean);
  return rows.map((row) => {
    const source = row.source.trim();
    const target = row.target.trim();
    if (target || !source) return { source, target };
    const name = storedNameFor(title, taken);
    taken.push(name);
    return { source, target: name };
  });
}

/** Where Browse opens: inside the prefix for a Windows game, the home folder otherwise. */
export function browseStart(locations: SaveLocations | null): string | undefined {
  if (!locations) return undefined;
  const preferred = locations.savesInPrefix
    ? (locations.prefixHome ?? locations.prefixDriveC)
    : locations.home;
  return preferred ?? locations.installDir ?? locations.home ?? undefined;
}

function isInside(path: string, root: string): boolean {
  return path === root || path.startsWith(`${root}/`) || path.startsWith(`${root}\\`);
}

/** A path as the game sees it: `C:\…` inside the prefix, `~/…` in the home folder. */
export function displayPath(path: string, locations: SaveLocations | null): string {
  const driveC = locations?.prefixDriveC;
  // Checked first: the prefix usually sits inside the home folder.
  if (driveC && isInside(path, driveC)) {
    return `C:${path.slice(driveC.length).replace(/\//g, "\\") || "\\"}`;
  }
  const home = locations?.home;
  if (home && isInside(path, home)) return `~${path.slice(home.length)}`;
  return path;
}
