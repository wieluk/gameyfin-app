/** Choosing among the setup programs that came with a game: the game, patches and DLC. */

import type { SetupProgram } from "@/bindings/SetupProgram";
import type { SetupRole } from "@/bindings/SetupRole";

export const ROLE_LABEL: Record<SetupRole, string> = {
  game: "Game",
  patch: "Patch",
  dlc: "DLC",
  other: "Other",
};

/** Ticked to start with: what an automatic install would run now. */
export function defaultSelection(setups: SetupProgram[]): string[] {
  return setups.filter((s) => s.recommended).map((s) => s.path);
}

/** Setups in the order the user arranged, any not yet arranged after them in plan order. */
export function arranged(setups: SetupProgram[], order: string[] | null): SetupProgram[] {
  if (!order) return setups;
  const rank = (path: string) => {
    const at = order.indexOf(path);
    return at === -1 ? order.length : at;
  };
  return [...setups].sort((a, b) => rank(a.path) - rank(b.path));
}

export function moved<T>(items: T[], from: number, to: number): T[] {
  const next = [...items];
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  return next;
}

/** The whole order after moving one shown row. Rows not shown keep their places. */
export function reordered(all: string[], shown: string[], from: number, to: number): string[] {
  const next = moved(shown, from, to);
  let at = 0;
  return all.map((path) => (shown.includes(path) ? next[at++] : path));
}

/** What runs: the ticked setups, in the arranged order. */
export function runOrder(setups: SetupProgram[], selected: string[]): string[] {
  return setups.filter((s) => selected.includes(s.path)).map((s) => s.path);
}

/** The selection with one setup switched, kept in the order they run. */
export function toggled(
  setups: SetupProgram[],
  selected: string[],
  path: string,
  on: boolean,
): string[] {
  return setups.map((s) => s.path).filter((p) => (p === path ? on : selected.includes(p)));
}

/** Whether nothing worth installing later is left out, so the download can go. */
export function coversEverything(setups: SetupProgram[], selected: string[]): boolean {
  return setups
    .filter((s) => !s.installed && !s.superseded && s.role !== "other")
    .every((s) => selected.includes(s.path));
}

/** Setups still worth running for an installed game. */
export function remaining(setups: SetupProgram[]): SetupProgram[] {
  return setups.filter((s) => !s.installed && !s.superseded && s.role !== "other");
}
