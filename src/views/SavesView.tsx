import { useCallback, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Empty } from "@/components/Empty";
import { Icon } from "@/components/Icon";
import { SaveConflictDialog } from "@/components/SaveConflictDialog";
import { SaveMatchDialog } from "@/components/SaveMatchDialog";
import { SavePathDialog } from "@/components/SavePathDialog";
import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { describe, isSaveState, outcomeOf } from "@/lib/saveState";
import { keys, useEntries, useInvalidate } from "@/lib/queries";
import { useTauriEvent } from "@/lib/useTauriEvent";
import type { ConflictChoice, LibraryEntry, SaveSyncState } from "@/types";

/** Saves only exist for a game that is actually installed here. */
function hasLocalInstall(entry: LibraryEntry): boolean {
  return entry.state.kind === "installed" || entry.state.kind === "running";
}

export function useSaveStates(entries: LibraryEntry[]) {
  return useQuery({
    queryKey: [...keys.saveStates, entries.map((e) => e.game.id).join(",")],
    enabled: entries.length > 0,
    queryFn: async () => {
      const states: Record<number, SaveSyncState> = {};
      // Sequential on purpose: each call may shell out to the backup helper, and a
      // library of a hundred games would otherwise spawn a hundred processes at once.
      for (const entry of entries) {
        try {
          states[entry.game.id] = await backend.saveState(entry.game.id);
        } catch (error) {
          states[entry.game.id] = { kind: "failed", message: messageOf(error) };
        }
      }
      return states;
    },
  });
}

export function SavesView() {
  const invalidate = useInvalidate();
  const entries = useEntries();
  const installed = useMemo(
    () => (entries.data ?? []).filter(hasLocalInstall),
    [entries.data],
  );
  const states = useSaveStates(installed);
  // A set, not one id: one game finishing must not re-enable every other row's buttons.
  const [busy, setBusy] = useState<ReadonlySet<number>>(new Set());
  const [conflictGameId, setConflictGameId] = useState<number | null>(null);
  const [identifyGameId, setIdentifyGameId] = useState<number | null>(null);
  const [pathsGameId, setPathsGameId] = useState<number | null>(null);

  const refresh = useCallback(() => {
    void invalidate(keys.saveStates);
  }, [invalidate]);

  useTauriEvent("save-state", refresh);

  // What the last press did, per game: without it a success and a failure look the same.
  const [outcome, setOutcome] = useState<Record<number, { text: string; ok: boolean }>>({});

  async function run(gameId: number, action: () => Promise<SaveSyncState | unknown>) {
    setBusy((current) => new Set(current).add(gameId));
    setOutcome((current) => {
      const { [gameId]: _gone, ...rest } = current;
      return rest;
    });
    try {
      const next = await action();
      if (isSaveState(next)) {
        setOutcome((current) => ({ ...current, [gameId]: outcomeOf(next) }));
      }
      refresh();
    } catch (error) {
      setOutcome((current) => ({
        ...current,
        [gameId]: { text: messageOf(error), ok: false },
      }));
    } finally {
      setBusy((current) => {
        const next = new Set(current);
        next.delete(gameId);
        return next;
      });
    }
  }

  const conflictEntry = installed.find((e) => e.game.id === conflictGameId);
  const conflictState = conflictGameId ? states.data?.[conflictGameId] : undefined;
  const identifyEntry = installed.find((e) => e.game.id === identifyGameId);
  const identifyState = identifyGameId ? states.data?.[identifyGameId] : undefined;
  const pathsEntry = installed.find((e) => e.game.id === pathsGameId);

  if (entries.isLoading || states.isLoading) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <div className="h-6 w-6 animate-spin rounded-full border-2 border-default-300 border-t-primary" />
      </div>
    );
  }

  if (installed.length === 0) {
    return (
      <Empty icon="cloud" title="No installed games yet">
        Saves are synced for games installed on this PC. Install one and play it, and its
        save will appear here.
      </Empty>
    );
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto p-6">
      <h1 className="mb-1 text-lg font-semibold">Saves</h1>
      <p className="mb-5 text-xs text-foreground/60">
        Your saves are backed up after you play and restored before you start, on every PC
        signed in to the same server.
      </p>

      <div className="flex flex-col gap-2">
        {installed.map((entry) => (
          <SaveRow
            key={entry.game.id}
            entry={entry}
            state={states.data?.[entry.game.id]}
            busy={busy.has(entry.game.id)}
            outcome={outcome[entry.game.id]}
            onBackup={() => run(entry.game.id, () => backend.backupSaves(entry.game.id, false))}
            onRestore={() => run(entry.game.id, () => backend.restoreSaves(entry.game.id))}
            onResolve={() => setConflictGameId(entry.game.id)}
            onEnableCrossOs={() =>
              run(entry.game.id, () => backend.setSaveCrossOs(entry.game.id, true))
            }
            onIdentify={() => setIdentifyGameId(entry.game.id)}
            onEditPaths={() => setPathsGameId(entry.game.id)}
          />
        ))}
      </div>

      {conflictEntry && conflictState?.kind === "conflict" && (
        <SaveConflictDialog
          title={conflictEntry.game.title}
          localAt={conflictState.localAt}
          remote={conflictState.remote}
          busy={busy.has(conflictEntry.game.id)}
          onCancel={() => setConflictGameId(null)}
          onChoose={(choice: ConflictChoice) => {
            const gameId = conflictEntry.game.id;
            setConflictGameId(null);
            void run(gameId, () => backend.resolveSaveConflict(gameId, choice));
          }}
        />
      )}

      {identifyEntry && (
        <SaveMatchDialog
          gameId={identifyEntry.game.id}
          gameTitle={identifyEntry.game.title}
          candidates={identifyState?.kind === "unmatched" ? identifyState.candidates : []}
          onClose={() => setIdentifyGameId(null)}
          onChosen={refresh}
        />
      )}

      {pathsEntry && (
        <SavePathDialog
          gameId={pathsEntry.game.id}
          gameTitle={pathsEntry.game.title}
          onClose={() => setPathsGameId(null)}
          onSaved={refresh}
        />
      )}
    </div>
  );
}

function SaveRow({
  entry,
  state,
  busy,
  onBackup,
  onRestore,
  onResolve,
  onEnableCrossOs,
  onIdentify,
  onEditPaths,
  outcome,
}: {
  entry: LibraryEntry;
  state?: SaveSyncState;
  busy: boolean;
  /** The result of the last button press, which the row state alone does not explain. */
  outcome?: { text: string; ok: boolean };
  onBackup: () => void;
  onRestore: () => void;
  onResolve: () => void;
  onEnableCrossOs: () => void;
  onIdentify: () => void;
  onEditPaths: () => void;
}) {
  const summary = describe(state);

  return (
    <div className="flex items-center gap-3 rounded-xl border border-default-200/60 bg-content1 px-4 py-3">
      <Icon name="cloud" className="h-4 w-4 shrink-0 text-foreground/40" />
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium">{entry.game.title}</p>
        <p className={`truncate text-xs ${summary.tone}`}>{summary.text}</p>
        {outcome && (
          <p
            role="status"
            className={`mt-0.5 text-[11px] leading-relaxed ${
              outcome.ok ? "text-success-600" : "text-warning-600"
            }`}
          >
            {outcome.text}
          </p>
        )}
      </div>

      <div className="flex shrink-0 gap-2">
        {state?.kind === "conflict" && (
          <Action label="Resolve" onClick={onResolve} busy={busy} primary />
        )}
        {state?.kind === "platform-mismatch" && state.crossOsAvailable && (
          <Action label="Try anyway" onClick={onEnableCrossOs} busy={busy} />
        )}
        {state?.kind === "remote-newer" && (
          <Action label="Restore" onClick={onRestore} busy={busy} primary />
        )}
        {(state?.kind === "local-newer" || state?.kind === "never-synced") && (
          <Action label="Back up" onClick={onBackup} busy={busy} primary />
        )}
        {state?.kind === "in-sync" && <Action label="Back up" onClick={onBackup} busy={busy} />}
        {/* The two states the user could previously do nothing about. */}
        {state?.kind === "unmatched" && (
          <>
            <Action label="Choose game" onClick={onIdentify} busy={busy} primary />
            {/* For a game in no version of the database, naming the folder is the only way. */}
            <Action label="Set folders" onClick={onEditPaths} busy={busy} />
          </>
        )}
        {state?.kind === "nothing-to-back-up" && (
          <>
            <Action label="Choose game" onClick={onIdentify} busy={busy} />
            <Action label="Set folders" onClick={onEditPaths} busy={busy} primary />
          </>
        )}
      </div>
    </div>
  );
}

function Action({
  label,
  onClick,
  busy,
  primary,
}: {
  label: string;
  onClick: () => void;
  busy: boolean;
  primary?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={busy}
      className={`rounded-lg px-3 py-1.5 text-xs font-medium disabled:opacity-50 ${
        primary
          ? "bg-primary text-white hover:bg-primary/90"
          : "bg-default-100 hover:bg-default-200"
      }`}
    >
      {busy ? "Working..." : label}
    </button>
  );
}

/** Games whose saves need the user to decide something, for the sidebar badge. */
export function countNeedingAttention(states?: Record<number, SaveSyncState>): number {
  if (!states) return 0;
  return Object.values(states).filter(
    (state) => state.kind === "conflict" || state.kind === "platform-mismatch",
  ).length;
}
