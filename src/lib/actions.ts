/**
 * The primary action for a game, given its state. Centralised so the tile, detail dialog
 * and Downloads list always agree.
 */

import { backend } from "./backend";
import type { GameState, LibraryEntry } from "@/types";

/** Placeholder for actions that are resolved by the chooser instead of run directly. */
const noop = async () => {};

export interface PrimaryAction {
  label: string;
  icon: "play" | "download" | "installed";
  /** Nothing to do, the game is mid-transfer or already running. */
  disabled: boolean;
  run: (gameId: number) => Promise<void>;
}

export function primaryAction(state: GameState): PrimaryAction {
  switch (state.kind) {
    case "installed":
      return { label: "Play", icon: "play", disabled: false, run: backend.launch };
    case "running":
      return { label: "Running", icon: "play", disabled: true, run: async () => {} };
    // These need a decision from the user, so the caller opens the chooser rather than
    // running anything. `run` is never reached for them, see `needsChooser`.
    case "downloaded":
      return { label: "Extract", icon: "installed", disabled: false, run: noop };
    case "extracted":
      return { label: "Install", icon: "installed", disabled: false, run: noop };
    case "downloading":
      return { label: "Downloading…", icon: "download", disabled: true, run: async () => {} };
    case "extracting":
      return { label: "Extracting…", icon: "installed", disabled: true, run: async () => {} };
    case "installing":
      return { label: "Installing…", icon: "installed", disabled: true, run: async () => {} };
    case "failed":
      // Retry whichever step failed rather than always restarting the download.
      return {
        label:
          state.stage === "install"
            ? "Retry install"
            : state.stage === "extract"
              ? "Retry extract"
              : "Retry download",
        icon: state.stage === "download" ? "download" : "installed",
        disabled: false,
        run: state.stage === "download" ? backend.startDownload : noop,
      };
    default:
      return { label: "Download", icon: "download", disabled: false, run: backend.startDownload };
  }
}

/** Whether the primary action needs the chooser rather than running directly. */
export function needsChooser(state: GameState): boolean {
  return (
    state.kind === "downloaded" ||
    state.kind === "extracted" ||
    (state.kind === "failed" && state.stage !== "download")
  );
}

export function isInstalled(entry: LibraryEntry): boolean {
  return entry.state.kind === "installed" || entry.state.kind === "running";
}

/** In the Downloads list: anything not yet a finished install. */
export function isInDownloads(entry: LibraryEntry): boolean {
  return [
    "downloading",
    "downloaded",
    "extracting",
    "extracted",
    "installing",
    "preparing",
    "failed",
  ].includes(entry.state.kind);
}
