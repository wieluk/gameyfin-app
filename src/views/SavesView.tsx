import { useCallback, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Alert } from "@/components/Alert";
import { Empty } from "@/components/Empty";
import { Icon } from "@/components/Icon";
import { Modal, ModalFooter, ModalHeader } from "@/components/Modal";
import { SaveConflictDialog } from "@/components/SaveConflictDialog";
import { SaveMatchDialog } from "@/components/SaveMatchDialog";
import { SavePathDialog, loadSavePaths } from "@/components/SavePathDialog";
import { usePreloaded } from "@/lib/usePreloaded";
import { SaveVersionList, platformLabel } from "@/components/SaveVersionList";
import { ScanThisPcDialog } from "@/components/ScanThisPcDialog";
import { OverflowMenu } from "@/components/OverflowMenu";
import { Button, IconButton, TextInput, ViewHeader } from "@/components/ui";
import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { formatBytes, formatRelative } from "@/lib/format";
import {
  attentionSummary,
  backsUp,
  byAttention,
  describe,
  isRestoreReport,
  isSaveState,
  outcomeOf,
  restoredOutcome,
} from "@/lib/saveState";
import { keys, useAppSettings, useEntries, useInvalidate } from "@/lib/queries";
import { readStored, writeStored } from "@/lib/storage";
import { PANEL_BODY } from "@/lib/ui";
import { useAction } from "@/lib/useAction";
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

/** What an empty list says, which depends on what it was asked to show. */
const EMPTY: Record<SaveScope, { title: string; text: string }> = {
  installed: {
    title: "No installed games yet",
    text: "Saves are synced for games installed on this PC. Show With saves for games with stored saves, or All games for your whole library.",
  },
  "with-saves": {
    title: "No saves yet",
    text: "Nothing is installed here and no saves are stored. Show All games for your whole library.",
  },
  all: { title: "No games yet", text: "Games appear here once your library has some." },
};

const SCOPE_KEY = "gameyfin.saves.scope";
/** The two switches the scope used to be, read once so an existing choice carries over. */
const OLD_SCOPE_KEYS = ["gameyfin.saves.show-all", "gameyfin.saves.show-all-games"] as const;

const SCOPES: [SaveScope, string][] = [
  ["installed", "Installed"],
  ["with-saves", "With saves"],
  ["all", "All games"],
];

function storedScope(): SaveScope {
  const [allSaves, allGames] = OLD_SCOPE_KEYS.map((key) => readStored(key, false));
  return readStored<SaveScope>(
    SCOPE_KEY,
    allGames ? "all" : allSaves ? "with-saves" : "installed",
  );
}

/** Where saves go, in the words the intro uses. */
const KEPT: Record<string, string> = {
  server: "on every PC signed in to the same server",
  folder: "on every PC that uses the same saves folder",
  webdav: "on every PC that uses the same WebDAV share",
};

export function SavesView() {
  const invalidate = useInvalidate();
  const settings = useAppSettings();
  const [scope, setScope] = useState<SaveScope>(storedScope);
  const [query, setQuery] = useState("");
  const overview = useSaveOverview(scope);
  const all = overview.data ?? [];
  const needle = query.trim().toLowerCase();
  const rows = byAttention(
    needle ? all.filter((row) => row.title.toLowerCase().includes(needle)) : all,
  );

  // A set, not one id: one game finishing must not re-enable every other row's buttons.
  const [busy, setBusy] = useState<ReadonlySet<number>>(new Set());
  const [conflictGameId, setConflictGameId] = useState<number | null>(null);
  const [identifyGameId, setIdentifyGameId] = useState<number | null>(null);
  // What is set is read first, so the dialog opens on it rather than filling in after.
  const paths = usePreloaded(loadSavePaths);
  // One at a time: two open version lists is two listings for no reason.
  const [expanded, setExpanded] = useState<number | null>(null);
  const [folderError, setFolderError] = useState<string | null>(null);
  const [scanning, setScanning] = useState(false);
  const [confirmDeleteAll, setConfirmDeleteAll] = useState(false);
  const [confirmBackupAll, setConfirmBackupAll] = useState(false);
  // Which game "Back up all" is on, shown in its dialog while it works through the list.
  const [backingUp, setBackingUp] = useState<{ done: number; total: number; title: string } | null>(
    null,
  );
  const [notice, setNotice] = useState<string | null>(null);
  const deleteAll = useAction();
  const queryClient = useQueryClient();

  async function deleteAllSaves() {
    const count = await deleteAll.run(() => backend.deleteAllSaves());
    if (count === undefined) return;
    setConfirmDeleteAll(false);
    setNotice(count === 1 ? "Deleted 1 save." : `Deleted ${count} saves.`);
    await queryClient.invalidateQueries({ queryKey: ["save-versions"] });
    await queryClient.invalidateQueries({ queryKey: keys.saveOverviewAll });
  }
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

  async function run(
    gameId: number,
    action: () => Promise<SaveSyncState | unknown>,
  ): Promise<boolean> {
    setBusy((current) => new Set(current).add(gameId));
    setOutcome((current) => {
      const { [gameId]: _gone, ...rest } = current;
      return rest;
    });
    try {
      const next = await action();
      let result: { text: string; ok: boolean } | null = null;
      if (isRestoreReport(next)) result = restoredOutcome(next);
      else if (isSaveState(next)) result = outcomeOf(next);
      if (result) {
        const shown = result;
        setOutcome((current) => ({ ...current, [gameId]: shown }));
      }
      refresh();
      return result?.ok ?? true;
    } catch (error) {
      setOutcome((current) => ({
        ...current,
        [gameId]: { text: messageOf(error), ok: false },
      }));
      return false;
    } finally {
      setBusy((current) => {
        const next = new Set(current);
        next.delete(gameId);
        return next;
      });
    }
  }

  function chooseScope(next: SaveScope) {
    setScope(next);
    writeStored(SCOPE_KEY, next);
  }

  // Every installed game, not only the rows a search leaves showing.
  const backupTargets = all.filter(backsUp);

  /** One game after another, as each backup runs the save helper over the whole game. */
  async function backupAll() {
    const failed: string[] = [];
    for (const [index, row] of backupTargets.entries()) {
      setBackingUp({ done: index, total: backupTargets.length, title: row.title });
      if (!(await run(row.gameId, () => backend.backupSaves(row.gameId, false)))) {
        failed.push(row.title);
      }
    }
    setBackingUp(null);
    setConfirmBackupAll(false);
    const done = backupTargets.length - failed.length;
    setNotice(
      `Backed up ${done} ${done === 1 ? "game" : "games"}.` +
        (failed.length === 0
          ? ""
          : ` Not finished: ${failed.join(", ")}. ${failed.length === 1 ? "Its row says" : "Their rows say"} why.`),
    );
  }

  const conflictRow = rows.find((row) => row.gameId === conflictGameId);
  const identifyRow = rows.find((row) => row.gameId === identifyGameId);
  const pathsRow = rows.find((row) => row.gameId === paths.opened?.target);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ViewHeader
        title="Saves"
        help="saves"
        actions={
          <>
            <TextInput
              icon="search"
              placeholder="Search saves"
              aria-label="Search saves"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              className="w-44"
            />
            <ScopePicker value={scope} onChange={chooseScope} />
            <Button onClick={() => setScanning(true)}>Find saves on this PC</Button>
            <OverflowMenu
              label="More save actions"
              items={[
                {
                  label: "Back up all…",
                  disabled: backupTargets.length === 0,
                  onSelect: () => {
                    setNotice(null);
                    setConfirmBackupAll(true);
                  },
                },
                { label: "Open backups folder", onSelect: () => void openSavesFolder() },
                {
                  label: "Delete all saves…",
                  destructive: true,
                  onSelect: () => {
                    setNotice(null);
                    setConfirmDeleteAll(true);
                  },
                },
              ]}
            />
          </>
        }
      />

      <div className={`${PANEL_BODY} flex flex-col`}>
        <p className="mb-3 text-xs text-foreground/60">
          Your saves are backed up after you play and restored before you start,{" "}
          {KEPT[settings.data?.saveBackend ?? "server"] ?? KEPT.server}.
          {scope === "with-saves" && " Games with stored saves are listed too, installed here or not."}
          {scope === "all" && " Every game in your library is listed, installed here or not."}
        </p>

        {all.length > 0 && (
          <p className="mb-4 text-xs font-medium text-foreground/80">
            {needle
              ? `${rows.length} of ${all.length} games match "${query.trim()}".`
              : attentionSummary(all)}
          </p>
        )}

        {notice && (
          <p role="status" className="mb-3 text-[0.6875rem] text-success-600">
            {notice}
          </p>
        )}

        {folderError && (
          <p role="alert" className="mb-3 text-[0.6875rem] text-warning-600">
            {folderError}
          </p>
        )}

        {overview.isLoading ? (
          <div className="flex flex-1 items-center justify-center">
            <div className="h-6 w-6 animate-spin rounded-full border-2 border-default-300 border-t-primary" />
          </div>
        ) : overview.error ? (
          <Alert>{messageOf(overview.error)}</Alert>
        ) : all.length === 0 ? (
          <Empty icon="cloud" title={EMPTY[scope].title}>
            {EMPTY[scope].text}
          </Empty>
        ) : rows.length === 0 ? (
          <Empty icon="search" title={`Nothing matches "${query.trim()}"`}>
            Only titles are searched.
            {scope !== "all" && " Show All games to search your whole library."}
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
                onEditPaths={() => void paths.open(row.gameId)}
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
      </div>

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

      {confirmBackupAll && (
        <Modal
          label="Back up all saves"
          role="alertdialog"
          size="sm"
          onDismiss={() => {
            if (!backingUp) setConfirmBackupAll(false);
          }}
        >
          <ModalHeader
            title={
              backingUp
                ? "Backing up saves"
                : `Back up ${backupTargets.length} ${backupTargets.length === 1 ? "game" : "games"}?`
            }
            description={
              backingUp
                ? `Backing up ${backingUp.title}, ${backingUp.done + 1} of ${backingUp.total}…`
                : "Every installed game whose saves Gameyfin has found is backed up, one after another. A save that has not changed is not uploaded again, and a game with a newer save elsewhere is left for you to decide."
            }
          />
          <ModalFooter>
            <Button
              variant="ghost"
              disabled={backingUp !== null}
              onClick={() => setConfirmBackupAll(false)}
            >
              Cancel
            </Button>
            <Button
              variant="primary"
              icon="cloud"
              disabled={backingUp !== null}
              onClick={() => void backupAll()}
            >
              {backingUp ? "Backing up…" : "Back up all"}
            </Button>
          </ModalFooter>
        </Modal>
      )}

      {confirmDeleteAll && (
        <Modal
          label="Delete all saves"
          role="alertdialog"
          size="sm"
          onDismiss={() => {
            if (!deleteAll.busy) setConfirmDeleteAll(false);
          }}
        >
          <ModalHeader
            title="Delete all saves?"
            description="Every stored save goes, for every game, including games not installed here. They cannot be recovered. The save files on this PC stay where they are."
          />
          {deleteAll.error && (
            <div className="px-5 pb-2">
              <Alert>{deleteAll.error}</Alert>
            </div>
          )}
          <ModalFooter>
            <Button
              variant="ghost"
              disabled={deleteAll.busy}
              onClick={() => setConfirmDeleteAll(false)}
            >
              Cancel
            </Button>
            <Button
              variant="danger"
              icon="close"
              disabled={deleteAll.busy}
              onClick={() => void deleteAllSaves()}
            >
              {deleteAll.busy ? "Deleting…" : "Delete all saves"}
            </Button>
          </ModalFooter>
        </Modal>
      )}

      {pathsRow && paths.opened && (
        <SavePathDialog
          gameId={pathsRow.gameId}
          gameTitle={pathsRow.title}
          initial={paths.opened.data}
          onClose={paths.close}
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
    <div className="rounded-xl border border-default-200 bg-content1">
      <div className="flex items-center gap-3 p-3">
        <IconButton
          icon="chevron"
          size="sm"
          label={expanded ? `Hide ${row.title}'s saves` : `Show ${row.title}'s saves`}
          aria-expanded={expanded}
          iconClassName={`h-4 w-4 transition-transform ${expanded ? "rotate-90" : ""}`}
          onClick={onToggle}
        />
        <Icon name="cloud" className="h-4 w-4 shrink-0 text-foreground/40" />
        <div className="min-w-0 flex-1">
          <p className="flex items-baseline gap-2 truncate text-sm font-medium">
            {row.title}
            {!row.installed && (
              <span className="shrink-0 text-[0.6875rem] font-normal text-foreground/40">
                not installed here
              </span>
            )}
          </p>
          <p className={`truncate text-xs ${summary.tone}`}>{summary.text}</p>
          {row.versions > 0 && (
            <p className="truncate text-[0.6875rem] text-foreground/40">
              {row.versions} {row.versions === 1 ? "version" : "versions"}
              {row.newestAt ? `, newest ${formatRelative(row.newestAt)}` : ""}
              {row.newestDevice ? ` from ${row.newestDevice}` : ""}
              {row.sizeBytes > 0 ? `, ${formatBytes(row.sizeBytes)}` : ""}
            </p>
          )}
          {outcome && (
            <p
              role="status"
              className={`mt-0.5 text-[0.6875rem] leading-relaxed ${
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
          {/* Not forced, so a retry stops at a conflict instead of overwriting a newer save. */}
          {state.kind === "failed" && row.installed && (
            <Action label="Try again" onClick={onBackup} busy={busy} />
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
              className="self-center text-[0.6875rem] text-foreground/40"
              title="A save is restored into the game's own folders, which only exist once it is installed."
            >
              Install to restore
            </span>
          )}
        </div>
      </div>

      {expanded && (
        <div className="flex flex-col gap-3 border-t border-default-200/60 px-3 py-3">
          <div>
            <p className="mb-1.5 text-xs text-foreground/55">Stored versions</p>
            <SaveVersionList
              gameId={row.gameId}
              busy={busy}
              canRestore={row.installed}
              onRestore={onRestoreVersion}
            />
          </div>

          {state.kind === "platform-mismatch" && (
            <p className="text-[0.6875rem] leading-relaxed text-warning-600">
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
            <Button icon="folder" onClick={onOpenFolder} disabled={busy}>
              Open folder
            </Button>
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
    <Button variant={primary ? "primary" : "secondary"} onClick={onClick} disabled={busy}>
      {busy ? "Working…" : label}
    </Button>
  );
}

/** Games whose saves need the user to decide something, for the sidebar badge. */
export function countNeedingAttention(rows?: SaveOverviewRow[]): number {
  if (!rows) return 0;
  return rows.filter(
    (row) => row.state.kind === "conflict" || row.state.kind === "platform-mismatch",
  ).length;
}

/** Which games the list shows: one choice of three, not two switches that depend on each other. */
function ScopePicker({ value, onChange }: { value: SaveScope; onChange: (scope: SaveScope) => void }) {
  return (
    <div
      role="group"
      aria-label="Show"
      className="flex shrink-0 items-center gap-0.5 rounded-lg border border-default-200 p-0.5"
    >
      {SCOPES.map(([scope, label]) => (
        <button
          key={scope}
          type="button"
          aria-pressed={value === scope}
          onClick={() => onChange(scope)}
          className={`rounded-md px-2.5 py-1 text-xs transition-colors ${
            value === scope
              ? "bg-primary/15 font-medium text-primary"
              : "text-foreground/60 hover:bg-default-100"
          }`}
        >
          {label}
        </button>
      ))}
    </div>
  );
}
