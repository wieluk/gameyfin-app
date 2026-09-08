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
        text: "No save files found for this game on this PC",
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
