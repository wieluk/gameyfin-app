import { useCallback, useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";

import { SaveVersionList } from "@/components/SaveVersionList";
import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { keys } from "@/lib/queries";
import { describe, isSaveState, outcomeOf } from "@/lib/saveState";
import { useTauriEvent } from "@/lib/useTauriEvent";
import type { LibraryEntry, SaveSyncState } from "@/types";

/** The states that mean this game has saves worth a section of its own. */
const SHOWN_STATES: SaveSyncState["kind"][] = [
  "in-sync",
  "local-newer",
  "remote-newer",
  "conflict",
  "platform-mismatch",
  "failed",
];

/**
 * A game's synced saves, inside its detail popup.
 *
 * Works against whichever location is configured, so it is just as useful when the server
 * has no save support and the user syncs to a folder or a WebDAV share instead.
 */
export function GameSaves({ entry }: { entry: LibraryEntry }) {
  const gameId = entry.game.id;
  const queryClient = useQueryClient();
  const [state, setState] = useState<SaveSyncState | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<{ text: string; ok: boolean } | null>(null);

  // Numbered, so a slow answer for the previous game cannot overwrite a newer one.
  const request = useRef(0);
  const load = useCallback(async () => {
    const mine = ++request.current;
    try {
      const next = await backend.saveState(gameId);
      if (mine !== request.current) return;
      setState(next);
      await queryClient.invalidateQueries({ queryKey: keys.saveVersions(gameId) });
    } catch (e) {
      if (mine === request.current) setState({ kind: "failed", message: messageOf(e) });
    }
  }, [gameId, queryClient]);

  useEffect(() => {
    void load();
  }, [load]);

  // The launch hooks back up and restore on their own, so the panel would otherwise go
  // stale while it is open.
  useTauriEvent("save-state", () => void load());

  async function act(action: () => Promise<unknown>) {
    setBusy(true);
    setError(null);
    setOutcome(null);
    try {
      const next = await action();
      // Say what the press achieved: the row's own state does not distinguish "backed up"
      // from "looked and found nothing".
      if (isSaveState(next)) setOutcome(outcomeOf(next));
      await load();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(false);
    }
  }

  // Nothing to say when saves cannot be synced at all, either here or by the server, or
  // when this game has none stored: the Saves tab is where a first backup is started, and
  // a line about a game with nothing to back up on every page is noise.
  if (!state || !SHOWN_STATES.includes(state.kind)) return null;

  const summary = describe(state);

  return (
    <section className="mt-6">
      <h3 className="mb-2 text-xs font-semibold uppercase tracking-wide text-foreground/45">
        Saves
      </h3>

      <div className="rounded-xl border border-default-200/60 bg-content1 px-4 py-3">
        <div className="flex items-center gap-3">
          <p className={`min-w-0 flex-1 text-xs ${summary.tone}`}>{summary.text}</p>
          <button
            type="button"
            disabled={busy}
            onClick={() => act(() => backend.backupSaves(gameId, false))}
            className="shrink-0 rounded-lg bg-default-100 px-3 py-1.5 text-xs font-medium hover:bg-default-200 disabled:opacity-50"
          >
            {busy ? "Working..." : "Back up now"}
          </button>
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

        <div className="mt-3 border-t border-default-200/60 pt-3">
          <SaveVersionList
            gameId={gameId}
            busy={busy}
            onRestore={(saveId) => act(() => backend.restoreSaves(gameId, saveId))}
          />
        </div>
      </div>
    </section>
  );
}
