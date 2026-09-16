/** When the save window around a launch or exit opens, changes or stays shut. Pure, so tested. */

import type { SaveSyncProgress } from "@/bindings/SaveSyncProgress";

/** Most checks finish inside this, and a window that flashes past cannot be read. */
export const SHOW_AFTER_MS = 1200;

export type Decision =
  | { kind: "show"; progress: SaveSyncProgress }
  /** Shown only if nothing else arrives first. */
  | { kind: "later"; progress: SaveSyncProgress }
  | { kind: "keep" }
  | { kind: "close" };

export function isFinal(progress: SaveSyncProgress): boolean {
  return ["done", "skipped", "kept-local", "restored", "failed", "asking"].includes(
    progress.phase.kind,
  );
}

/** Worth a window even when no progress was on screen. */
export function isInformative(progress: SaveSyncProgress): boolean {
  const phase = progress.phase;
  switch (phase.kind) {
    case "failed":
    case "restored":
      return true;
    // An exit upload held back means a decision waits under Saves.
    case "skipped":
      return progress.moment === "exit";
    case "done":
      return ["conflict", "failed", "platform-mismatch"].includes(phase.state.kind);
    default:
      return false;
  }
}

export function decide(shown: SaveSyncProgress | null, next: SaveSyncProgress): Decision {
  // The save prompt takes over: two dialogs about one save is one too many.
  if (next.phase.kind === "asking") {
    return shown && shown.gameId !== next.gameId ? { kind: "keep" } : { kind: "close" };
  }
  if (!isFinal(next)) {
    return shown && !isFinal(shown)
      ? { kind: "show", progress: next }
      : { kind: "later", progress: next };
  }
  if (isInformative(next)) return { kind: "show", progress: next };
  // A quiet outcome closes out a window already open, opens none, and covers nothing worth reading.
  if (!shown || isInformative(shown)) return { kind: "keep" };
  return { kind: "show", progress: next };
}
