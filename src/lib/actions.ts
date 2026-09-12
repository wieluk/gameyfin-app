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
      // Setting up a prefix, or running an installer: installed, just not startable yet.
      if (state.busy && state.busy.kind !== "failed") {
        return {
          label: state.busy.kind === "preparing" ? "Setting up…" : "Installing…",
          icon: "play",
          disabled: true,
          run: async () => {},
        };
      }
      return { label: "Play", icon: "play", disabled: false, run: backend.launch };
    case "running":
      return { label: "Running", icon: "play", disabled: true, run: async () => {} };
    // These need a decision, so the caller opens the chooser (see `needsChooser`). Not
    // labelled "Extract": what the download holds decides that.
    case "downloaded":
      return { label: "Install", icon: "installed", disabled: false, run: noop };
    case "extracted":
      return { label: "Install", icon: "installed", disabled: false, run: noop };
    case "downloading":
      return { label: "Downloading…", icon: "download", disabled: true, run: async () => {} };
    case "extracting":
      return { label: "Extracting…", icon: "installed", disabled: true, run: async () => {} };
    case "installing":
      return { label: "Installing…", icon: "installed", disabled: true, run: async () => {} };
    case "failed":
      // Retry the step that failed. A launch failure offers Play: the game is installed,
      // and re-downloading it was never the fix.
      if (state.stage === "launch") {
        return { label: "Play", icon: "play", disabled: false, run: backend.launch };
      }
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
  // A game that failed to *start* is still installed. Its files are on disk and the fix
  // is to try again or pick another executable, neither of which is in Downloads.
  return (
    entry.state.kind === "installed" ||
    entry.state.kind === "running" ||
    (entry.state.kind === "failed" && entry.state.stage === "launch")
  );
}

/**
 * The installed files a row needs, after a failed launch as well as before one: that state
 * carries none of the install details, and the row would grey out with no way back.
 */
export function installedFiles(state: GameState): {
  path: string | null;
  executable: string | null;
  setupCandidates: string[];
  stagingSetups: string[];
  stagingPresent: boolean;
} {
  const none = {
    path: null,
    executable: null,
    setupCandidates: [],
    stagingSetups: [],
    stagingPresent: false,
  };
  if (state.kind === "installed") {
    return {
      path: state.path,
      executable: state.executable ?? null,
      setupCandidates: state.setupCandidates ?? [],
      stagingSetups: state.stagingSetups ?? [],
      stagingPresent: Boolean(state.stagingPresent),
    };
  }
  if (state.kind === "failed" && state.stage === "launch") {
    return { ...none, path: state.path ?? null, executable: state.executable ?? null };
  }
  return none;
}

/**
 * Whether anything of this game is on this machine. Narrower than `isInDownloads`: a failed
 * empty download or a prefix being prepared has nothing playable offline.
 */
export function isLocal(entry: LibraryEntry): boolean {
  if (entry.state.kind === "failed") return entry.state.stage === "launch";
  return ["installed", "running", "downloaded", "extracted"].includes(entry.state.kind);
}

/** In the Downloads list: anything not yet a finished install. */
export function isInDownloads(entry: LibraryEntry): boolean {
  // A launch failure is the exception: getting there means the install finished.
  if (entry.state.kind === "failed") return entry.state.stage !== "launch";
  return [
    "downloading",
    "downloaded",
    "extracting",
    "extracted",
    "installing",
    "preparing",
  ].includes(entry.state.kind);
}
