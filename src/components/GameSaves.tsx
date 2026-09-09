import { useCallback, useEffect, useState } from "react";
import { backend, isMockBackend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { formatBytes, formatRelative } from "@/lib/format";
import { describe, outcomeOf } from "@/lib/saveState";
import type { LibraryEntry, SaveSyncState, SaveVersion } from "@/types";

/**
 * A game's synced saves, inside its detail popup.
 *
 * Works against whichever location is configured, so it is just as useful when the server
 * has no save support and the user syncs to a folder or a WebDAV share instead.
 */
export function GameSaves({ entry }: { entry: LibraryEntry }) {
  const gameId = entry.game.id;
  const [state, setState] = useState<SaveSyncState | null>(null);
  const [versions, setVersions] = useState<SaveVersion[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<{ text: string; ok: boolean } | null>(null);

  const load = useCallback(async () => {
    try {
      const next = await backend.saveState(gameId);
      setState(next);
      // Only the states that imply something is actually stored are worth listing for.
      setVersions(
        ["in-sync", "local-newer", "remote-newer", "conflict"].includes(next.kind)
          ? await backend.listSaveVersions(gameId)
          : [],
      );
    } catch (e) {
      setState({ kind: "failed", message: messageOf(e) });
    }
  }, [gameId]);

  useEffect(() => {
    void load();
  }, [load]);

  // The launch hooks back up and restore on their own, so the panel would otherwise go
  // stale while it is open.
  useEffect(() => {
    if (isMockBackend) return;
    let unlisten: (() => void) | undefined;
    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      unlisten = await listen("save-state", () => void load());
    })();
    return () => unlisten?.();
  }, [load]);

  async function act(action: () => Promise<unknown>) {
    setBusy(true);
    setError(null);
    setOutcome(null);
    try {
      const next = await action();
      // Say what the press achieved: the row's own state does not distinguish "backed up"
      // from "looked and found nothing".
      if (typeof next === "object" && next !== null && "kind" in next) {
        setOutcome(outcomeOf(next as SaveSyncState));
      }
      await load();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(false);
    }
  }

  // Nothing to say about a game with no save support anywhere.
  if (!state || state.kind === "unsupported") return null;

  const summary = describe(state);

  return (
    <section className="mt-6">
      <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-foreground/45">
        Saves
      </h3>

      <div className="rounded-xl border border-default-200/60 bg-content1 px-4 py-3">
        <div className="flex items-center gap-3">
          <p className={`min-w-0 flex-1 text-xs ${summary.tone}`}>{summary.text}</p>
          {state.kind !== "disabled" && (
            <button
              type="button"
              disabled={busy}
              onClick={() => act(() => backend.backupSaves(gameId, false))}
              className="shrink-0 rounded-lg bg-default-100 px-3 py-1.5 text-xs font-medium hover:bg-default-200 disabled:opacity-50"
            >
              {busy ? "Working..." : "Back up now"}
            </button>
          )}
        </div>

        {error && <p className="mt-2 text-xs text-danger">{error}</p>}
        {outcome && !error && (
          <p
            role="status"
            className={`mt-2 text-xs leading-relaxed ${
              outcome.ok ? "text-success-600" : "text-warning-600"
            }`}
          >
            {outcome.text}
          </p>
        )}

        {versions.length > 0 && (
          <ul className="mt-3 flex flex-col gap-1 border-t border-default-200/60 pt-3">
            {versions.map((version) => (
              <li key={version.id} className="flex items-center gap-3 text-[11px]">
                <span className="min-w-0 flex-1 truncate text-foreground/70">
                  {formatRelative(version.createdAt)}
                  {version.deviceName ? ` from ${version.deviceName}` : ""}
                </span>
                <span className="shrink-0 text-foreground/40">
                  {formatBytes(version.sizeBytes)}
                </span>
                {version.locked && (
                  <span className="shrink-0 text-foreground/40" title="Kept forever">
                    kept
                  </span>
                )}
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => act(() => backend.restoreSaves(gameId, version.id))}
                  className="shrink-0 rounded px-2 py-0.5 text-foreground/60 hover:bg-default-100 disabled:opacity-50"
                >
                  Restore
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}
