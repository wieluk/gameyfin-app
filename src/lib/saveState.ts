import { formatRelative } from "@/lib/format";
import type { RestoreReport } from "@/bindings/RestoreReport";
import type { SaveSyncState } from "@/types";

/** Narrows the value a save command returned; the others answer with nothing useful. */
export function isSaveState(value: unknown): value is SaveSyncState {
  return typeof value === "object" && value !== null && "kind" in value;
}

export function isRestoreReport(value: unknown): value is RestoreReport {
  return typeof value === "object" && value !== null && "folders" in value && "state" in value;
}

/** One line on what a restore put back and where, so the save can be found afterwards. */
export function restoredText(restored: Omit<RestoreReport, "state">): string {
  const made = `made ${formatRelative(restored.savedAt)}${restored.device ? ` on ${restored.device}` : ""}`;
  if (restored.files === 0) return `Restored the save ${made}, but it held no files.`;
  const files = `${restored.files} ${restored.files === 1 ? "file" : "files"}`;
  return `Restored the save ${made}: ${files} to ${restored.folders.join(", ")}.`;
}

/** What to tell the user after pressing Restore. A restore that wrote nothing says why. */
export function restoredOutcome(report: RestoreReport): { text: string; ok: boolean } {
  if (report.state.kind !== "in-sync") return outcomeOf(report.state);
  return { text: restoredText(report), ok: report.files > 0 };
}

/** One line saying where a game stands, and how alarmed to look about it. */
export function describe(state?: SaveSyncState): { text: string; tone: string } {
  const muted = "text-foreground/50";

  switch (state?.kind) {
    case undefined:
      return { text: "Checking...", tone: muted };
    case "off":
      return { text: "Save sync is switched off in Settings", tone: muted };
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
        text: `Only saved on ${platformName(state.remote)}, which does not restore onto ${platformName(state.local)} by itself`,
        tone: "text-warning-600",
      };
    case "failed":
      return { text: state.message, tone: "text-danger" };
  }
}

/** The stored tag is an enum on the wire; nobody says "PROTON" out loud. */
export function platformLabel(platform: string): string {
  switch (platform) {
    case "WINDOWS":
      return "Windows";
    case "LINUX":
      return "Linux";
    case "PROTON":
      return "Windows, on Proton";
    case "MACOS":
      return "macOS";
    default:
      return "";
  }
}

function platformName(platform: string): string {
  return platformLabel(platform) || "another system";
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
    case "off":
      return { text: "Save sync is switched off in Settings.", ok: false };
    case "unsupported":
      return { text: "This server cannot store saves.", ok: false };
    case "disabled":
      return { text: "Save sync is turned off on the server.", ok: false };
    case "failed":
      return { text: state.message, ok: false };
    case "platform-mismatch":
      return { text: describe(state).text, ok: false };
    default:
      return { text: describe(state).text, ok: true };
  }
}

/** What a row in the Saves list needs to sort and summarise it. */
type Listed = { state: SaveSyncState; installed: boolean };

/** How soon a row needs the user, lowest first. A row needing nothing sorts last. */
export function attentionRank({ state, installed }: Listed): number {
  switch (state.kind) {
    case "conflict":
      return 0;
    case "platform-mismatch":
      return 1;
    case "failed":
      return 2;
    case "remote-newer":
      return installed ? 3 : 6;
    case "unmatched":
      return 4;
    case "local-newer":
    case "never-synced":
      return installed ? 5 : 6;
    default:
      return 6;
  }
}

/** Attention first, the server's order kept within each group. */
export function byAttention<T extends Listed>(rows: T[]): T[] {
  return rows
    .map((row, index) => ({ row, index, rank: attentionRank(row) }))
    .sort((a, b) => a.rank - b.rank || a.index - b.index)
    .map(({ row }) => row);
}

const ATTENTION: [rank: number, one: string, many: (n: number) => string][] = [
  [0, "1 conflict to settle", (n) => `${n} conflicts to settle`],
  [1, "1 saved on another system", (n) => `${n} saved on another system`],
  [2, "1 failed", (n) => `${n} failed`],
  [3, "1 newer save to restore", (n) => `${n} newer saves to restore`],
  [4, "1 game to identify", (n) => `${n} games to identify`],
  [5, "1 not backed up yet", (n) => `${n} not backed up yet`],
];

/** One line on what needs doing, such as "1 newer save to restore, 1 failed". */
export function attentionSummary(rows: Listed[]): string {
  const games = `${rows.length} ${rows.length === 1 ? "game" : "games"}`;
  const parts = ATTENTION.flatMap(([rank, one, many]) => {
    const count = rows.filter((row) => attentionRank(row) === rank).length;
    return count === 0 ? [] : [count === 1 ? one : many(count)];
  });
  return parts.length === 0 ? `${games}. Nothing needs you.` : `${games}. ${parts.join(", ")}.`;
}

/** Whether "Back up all" includes a row: installed, its saves found, and nothing to settle. */
export function backsUp({ state, installed }: Listed): boolean {
  return (
    installed &&
    ["in-sync", "local-newer", "never-synced", "failed", "platform-mismatch"].includes(state.kind)
  );
}
