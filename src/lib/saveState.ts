import { formatRelative } from "@/lib/format";
import type { SaveSyncState } from "@/types";

/** One line saying where a game stands, and how alarmed to look about it. */
export function describe(state?: SaveSyncState): { text: string; tone: string } {
  const muted = "text-foreground/50";

  switch (state?.kind) {
    case undefined:
      return { text: "Checking...", tone: muted };
    case "unsupported":
      return { text: "Your server does not support save sync", tone: muted };
    case "disabled":
      return { text: "Save sync is turned off on your server", tone: muted };
    case "unmatched":
      return {
        text:
          state.candidates.length > 0
            ? `Not sure which game this is. Closest: ${state.candidates[0]}`
            : "This game is not in the save location database",
        tone: "text-warning-600",
      };
    case "never-synced":
      return { text: "Not backed up yet", tone: muted };
    case "nothing-to-back-up":
      return {
        text: state.known
          ? `Nothing saved yet, or the saves are not where "${state.title}" keeps them`
          : `"${state.title}" is not in the save location database`,
        tone: "text-warning-600",
      };
    case "in-sync":
      return { text: `Backed up ${formatRelative(state.lastSyncedAt)}`, tone: muted };
    case "local-newer":
      return { text: `Played ${formatRelative(state.localAt)}, not uploaded yet`, tone: muted };
    case "remote-newer":
      return {
        text: `A newer save from ${state.device ?? "another PC"}, ${formatRelative(state.remoteAt)}`,
        tone: "text-primary",
      };
    case "conflict":
      return {
        text: `This PC and ${state.remote.deviceName ?? "another PC"} both have unsaved progress`,
        tone: "text-warning-600",
      };
    case "platform-mismatch":
      return {
        text: state.crossOsAvailable
          ? `Saved on ${state.remote}, which does not map onto ${state.local} on its own`
          : `Saved on ${state.remote} and cannot be restored on ${state.local}`,
        tone: "text-warning-600",
      };
    case "failed":
      return { text: state.message, tone: "text-danger" };
  }
}

/** What to tell the user right after they pressed a button, as opposed to the row's state. */
export function outcomeOf(state: SaveSyncState): { text: string; ok: boolean } {
  switch (state.kind) {
    case "in-sync":
      return { text: "Backed up.", ok: true };
    case "local-newer":
      return { text: "Backed up. It has not reached the other end yet.", ok: true };
    case "nothing-to-back-up":
      return {
        text: state.known
          ? `Ludusavi looked where "${state.title}" keeps its saves and found no files. ` +
            "If you have played it, use Set folders to say where the saves are."
          : `Ludusavi has no entry for "${state.title}". Update the game database in ` +
            "Settings, or use Set folders to say where the saves are.",
        ok: false,
      };
    case "unmatched":
      return {
        text: "Could not tell which game this is. Use Choose game, or Set folders.",
        ok: false,
      };
    case "conflict":
      return { text: "Another device has a save this one did not start from.", ok: false };
    case "unsupported":
      return { text: "This server cannot store saves.", ok: false };
    case "disabled":
      return { text: "Save sync is turned off on the server.", ok: false };
    case "failed":
      return { text: state.message, ok: false };
    default:
      return { text: describe(state).text, ok: true };
  }
}
