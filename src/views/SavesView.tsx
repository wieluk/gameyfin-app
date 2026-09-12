import { useCallback, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Empty } from "@/components/Empty";
import { Icon } from "@/components/Icon";
import { SaveConflictDialog } from "@/components/SaveConflictDialog";
import { SaveMatchDialog } from "@/components/SaveMatchDialog";
import { SavePathDialog } from "@/components/SavePathDialog";
import { SaveVersionList, platformLabel } from "@/components/SaveVersionList";
import { ScanThisPcDialog } from "@/components/ScanThisPcDialog";
import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { formatBytes, formatRelative } from "@/lib/format";
import { describe, isSaveState, outcomeOf } from "@/lib/saveState";
import { keys, useEntries, useInvalidate } from "@/lib/queries";
import { readStored, writeStored } from "@/lib/storage";
import { useTauriEvent } from "@/lib/useTauriEvent";
import type { ConflictChoice, SaveSyncState } from "@/types";
import type { SaveOverviewRow } from "@/bindings/SaveOverviewRow";
import type { SaveScope } from "@/bindings/SaveScope";

/** Every game's save status in one request, which never runs the backup helper. */
export function useSaveOverview(scope: SaveScope) {
  return useQuery({
    queryKey: keys.saveOverview(scope),
    queryFn: () => backend.saveOverview(scope),
  });
}

const SHOW_ALL_KEY = "gameyfin.saves.show-all";

export function SavesView() {
  const invalidate = useInvalidate();
  const [showAll, setShowAll] = useState(() => readStored(SHOW_ALL_KEY, false));
  const scope: SaveScope = showAll ? "all" : "installed";
  const overview = useSaveOverview(scope);
  const rows = overview.data ?? [];

  // A set, not one id: one game finishing must not re-enable every other row's buttons.
  const [busy, setBusy] = useState<ReadonlySet<number>>(new Set());
  const [conflictGameId, setConflictGameId] = useState<number | null>(null);
  const [identifyGameId, setIdentifyGameId] = useState<number | null>(null);
  const [pathsGameId, setPathsGameId] = useState<number | null>(null);
  // One at a time: two open version lists is two listings for no reason.
  const [expanded, setExpanded] = useState<number | null>(null);
  const [folderError, setFolderError] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const entries = useEntries();

  /** The saves folder itself, which is the same for every game on this PC. */
  async function openSavesFolder() {
    setFolderError(null);
    try {
      const where = await backend.saveLocations(rows[0]?.gameId ?? 0, false);
      if (where.savesRoot) await backend.openFolder(where.savesRoot);
      else setFolderError("Nothing has been backed up on this PC yet.");
    } catch (e) {
      setFolderError(messageOf(e));
    }
  }

  /** Where this game's saves actually are, which needs a scan to answer. */
  async function openGameSaves(gameId: number) {
    setFolderError(null);
    try {
      const where = await backend.saveLocations(gameId, true);
      const folder =
        where.detected[0] ?? where.prefixHome ?? where.staging ?? where.savesRoot ?? null;
      if (folder) await backend.openFolder(folder);
      else setFolderError("No save folder has been found for this game yet.");
    } catch (e) {
      setFolderError(messageOf(e));
    }
  }

  const refresh = useCallback(() => {
    void invalidate(keys.saveOverviewAll);
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

  function chooseScope(checked: boolean) {
    setShowAll(checked);
    writeStored(SHOW_ALL_KEY, checked);
  }

  const conflictRow = rows.find((row) => row.gameId === conflictGameId);
  const identifyRow = rows.find((row) => row.gameId === identifyGameId);
  const pathsRow = rows.find((row) => row.gameId === pathsGameId);

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-y-auto p-6">
      <div className="mb-1 flex items-baseline justify-between gap-4">
        <h1 className="text-lg font-semibold">Saves</h1>
        <div className="flex shrink-0 items-center gap-4">
          <button
            type="button"
            onClick={() => setScanning(true)}
            className="text-xs text-foreground/60 underline-offset-2 transition-colors hover:text-foreground hover:underline"
          >
            Find saves on this PC
          </button>
          <button
            type="button"
            onClick={() => void openSavesFolder()}
            className="text-xs text-foreground/60 underline-offset-2 transition-colors hover:text-foreground hover:underline"
          >
            Open saves folder
          </button>
          <label className="flex cursor-pointer items-center gap-2 text-xs text-foreground/60 transition-colors hover:text-foreground">
            <input
              type="checkbox"
              checked={showAll}
              onChange={(e) => chooseScope(e.target.checked)}
              className="h-3.5 w-3.5 accent-primary"
            />
            Show all saves
          </label>
        </div>
      </div>
      <p className="mb-5 text-xs text-foreground/60">
        Your saves are backed up after you play and restored before you start, on every PC
        signed in to the same server.
        {showAll && " Every game in your library is listed, installed here or not."}
      </p>

      {folderError && (
        <p role="alert" className="mb-3 text-[11px] text-warning-600">
          {folderError}
        </p>
      )}

      {overview.isLoading ? (
        <div className="flex flex-1 items-center justify-center">
          <div className="h-6 w-6 animate-spin rounded-full border-2 border-default-300 border-t-primary" />
        </div>
      ) : overview.error ? (
        <p role="alert" className="text-xs text-danger">
          {messageOf(overview.error)}
        </p>
      ) : rows.length === 0 ? (
        <Empty icon="cloud" title={showAll ? "No games yet" : "No installed games yet"}>
          {showAll
            ? "Games appear here once your library has some."
            : "Saves are synced for games installed on this PC. Install one and play it, and its save will appear here. Tick Show all saves to include the rest of your library."}
        </Empty>
      ) : (
        <div className="flex flex-col gap-2">
          {rows.map((row) => (
            <SaveRow
              key={row.gameId}
              row={row}
              busy={busy.has(row.gameId)}
              outcome={outcome[row.gameId]}
              onBackup={() => run(row.gameId, () => backend.backupSaves(row.gameId, false))}
              onRestore={() => run(row.gameId, () => backend.restoreSaves(row.gameId))}
              onResolve={() => setConflictGameId(row.gameId)}
              onEnableCrossOs={() =>
                run(row.gameId, () => backend.setSaveCrossOs(row.gameId, true))
              }
              onIdentify={() => setIdentifyGameId(row.gameId)}
              onEditPaths={() => setPathsGameId(row.gameId)}
              expanded={expanded === row.gameId}
              onToggle={() =>
                setExpanded((current) => (current === row.gameId ? null : row.gameId))
              }
              onRestoreVersion={(saveId) =>
                run(row.gameId, () => backend.restoreSaves(row.gameId, saveId))
              }
              onOpenFolder={() => void openGameSaves(row.gameId)}
            />
          ))}
        </div>
      )}

      {conflictRow && conflictRow.state.kind === "conflict" && (
        <SaveConflictDialog
          title={conflictRow.title}
          localAt={conflictRow.state.localAt}
          remote={conflictRow.state.remote}
          busy={busy.has(conflictRow.gameId)}
          onCancel={() => setConflictGameId(null)}
          onChoose={(choice: ConflictChoice) => {
            const gameId = conflictRow.gameId;
            setConflictGameId(null);
            void run(gameId, () => backend.resolveSaveConflict(gameId, choice));
          }}
        />
      )}

      {identifyRow && (
        <SaveMatchDialog
          gameId={identifyRow.gameId}
          gameTitle={identifyRow.title}
          candidates={
            identifyRow.state.kind === "unmatched" ? identifyRow.state.candidates : []
          }
          onClose={() => setIdentifyGameId(null)}
          onChosen={refresh}
        />
      )}

      {scanning && (
        <ScanThisPcDialog
          games={(entries.data ?? []).map((entry) => entry.game)}
          onClose={() => setScanning(false)}
          onDone={refresh}
        />
      )}

      {pathsRow && (
        <SavePathDialog
          gameId={pathsRow.gameId}
          gameTitle={pathsRow.title}
          onClose={() => setPathsGameId(null)}
          onSaved={refresh}
        />
      )}
    </div>
  );
}

function SaveRow({
  row,
  busy,
  onBackup,
  onRestore,
  onResolve,
  onEnableCrossOs,
  onIdentify,
  onEditPaths,
  outcome,
  expanded,
  onToggle,
  onRestoreVersion,
  onOpenFolder,
}: {
  row: SaveOverviewRow;
  busy: boolean;
  /** The result of the last button press, which the row state alone does not explain. */
  outcome?: { text: string; ok: boolean };
  onBackup: () => void;
  onRestore: () => void;
  onResolve: () => void;
  onEnableCrossOs: () => void;
  onIdentify: () => void;
  onEditPaths: () => void;
  expanded: boolean;
  onToggle: () => void;
  onRestoreVersion: (saveId: string) => void;
  onOpenFolder: () => void;
}) {
  const state = row.state;
  const summary = describe(state);

  return (
    <div className="rounded-xl border border-default-200/60 bg-content1">
      <div className="flex items-center gap-3 px-4 py-3">
        <button
          type="button"
          onClick={onToggle}
          aria-expanded={expanded}
          aria-label={expanded ? `Hide ${row.title}'s saves` : `Show ${row.title}'s saves`}
          className="shrink-0 rounded p-0.5 text-foreground/40 transition-colors hover:text-foreground"
        >
          <Icon
            name="chevron"
            className={`h-3.5 w-3.5 transition-transform ${expanded ? "rotate-90" : ""}`}
          />
        </button>
        <Icon name="cloud" className="h-4 w-4 shrink-0 text-foreground/40" />
        <div className="min-w-0 flex-1">
          <p className="flex items-baseline gap-2 truncate text-sm font-medium">
            {row.title}
            {!row.installed && (
              <span className="shrink-0 text-[11px] font-normal text-foreground/40">
                not installed here
              </span>
            )}
          </p>
          <p className={`truncate text-xs ${summary.tone}`}>{summary.text}</p>
          {row.versions > 0 && (
            <p className="truncate text-[11px] text-foreground/40">
              {row.versions} {row.versions === 1 ? "version" : "versions"}
              {row.newestAt ? `, newest ${formatRelative(row.newestAt)}` : ""}
              {row.newestDevice ? ` from ${row.newestDevice}` : ""}
              {row.sizeBytes > 0 ? `, ${formatBytes(row.sizeBytes)}` : ""}
            </p>
          )}
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
          {state.kind === "conflict" && (
            <Action label="Resolve" onClick={onResolve} busy={busy} primary />
          )}
          {state.kind === "platform-mismatch" && state.crossOsAvailable && (
            <Action label="Try anyway" onClick={onEnableCrossOs} busy={busy} />
          )}
          {state.kind === "remote-newer" && row.installed && (
            <Action label="Restore" onClick={onRestore} busy={busy} primary />
          )}
          {(state.kind === "local-newer" || state.kind === "never-synced") && row.installed && (
            <Action label="Back up" onClick={onBackup} busy={busy} primary />
          )}
          {state.kind === "in-sync" && row.installed && (
            <Action label="Back up" onClick={onBackup} busy={busy} />
          )}
          {state.kind === "unmatched" && (
            <>
              <Action label="Choose game" onClick={onIdentify} busy={busy} primary />
              {/* For a game in no version of the database, naming the folder is the only way. */}
              <Action label="Set folders" onClick={onEditPaths} busy={busy} />
            </>
          )}
          {state.kind === "nothing-to-back-up" && (
            <>
              <Action label="Choose game" onClick={onIdentify} busy={busy} />
              <Action label="Set folders" onClick={onEditPaths} busy={busy} primary />
            </>
          )}
          {!row.installed && row.versions > 0 && (
            <span
              className="self-center text-[11px] text-foreground/40"
              title="A save is restored into the game's own folders, which only exist once it is installed."
            >
              Install to restore
            </span>
          )}
        </div>
      </div>

      {expanded && (
        <div className="flex flex-col gap-3 border-t border-default-200/60 px-4 py-3">
          <div>
            <p className="mb-1.5 text-[11px] font-medium text-foreground/45">Stored versions</p>
            <SaveVersionList
              gameId={row.gameId}
              busy={busy}
              canRestore={row.installed}
              onRestore={onRestoreVersion}
            />
          </div>

          {state.kind === "platform-mismatch" && (
            <p className="text-[11px] leading-relaxed text-warning-600">
              Saved on {platformLabel(state.remote) || state.remote}, and this PC plays it as{" "}
              {platformLabel(state.local) || state.local}.
              {state.crossOsAvailable
                ? " Try anyway translates the paths between the two. It is best effort, and it does not carry registry settings."
                : " Set the save folders by hand to say where they belong here."}
            </p>
          )}

          {/* Reachable whatever the state: a game syncing to the wrong place looks fine
              from the outside, and these two are how that gets fixed. */}
          <div className="flex flex-wrap gap-2">
            <Action label="Open folder" onClick={onOpenFolder} busy={busy} />
            <Action label="Set folders" onClick={onEditPaths} busy={busy} />
            <Action label="Choose game" onClick={onIdentify} busy={busy} />
            {state.kind === "conflict" && (
              <Action label="Resolve" onClick={onResolve} busy={busy} />
            )}
          </div>
        </div>
      )}
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
export function countNeedingAttention(rows?: SaveOverviewRow[]): number {
  if (!rows) return 0;
  return rows.filter(
    (row) => row.state.kind === "conflict" || row.state.kind === "platform-mismatch",
  ).length;
}
