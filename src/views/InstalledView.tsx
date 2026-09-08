import { useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { FolderActions } from "@/components/FolderActions";
import { GameOptions } from "@/components/GameOptions";
import { Icon } from "@/components/Icon";
import { ShortcutOptions } from "@/components/ShortcutOptions";
import { UninstallDialog } from "@/components/UninstallDialog";
import { isInstalled } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { formatPlaytime } from "@/lib/format";
import { messageOf } from "@/lib/errors";
import type { LibraryEntry } from "@/types";
import { Empty } from "@/components/Empty";
import { useEntries } from "@/lib/queries";
import { useRescanOnOpen } from "@/lib/rescan";

/** Games actually present on this machine and ready to play. */
export function InstalledView() {
  const entries = useEntries();
  const [error, setError] = useState<string | null>(null);

  useRescanOnOpen();

  const header = (
    <>
      <div className="flex shrink-0 items-center justify-between border-b border-default-200/60 px-6 py-3">
        <h2 className="text-xs font-semibold uppercase tracking-wide text-foreground/45">
          Installed
        </h2>
        <div className="flex items-center gap-3">
          <FolderActions folder="installations" onError={setError} />
        </div>
      </div>
      {error && (
        <p
          role="alert"
          className="mx-6 mt-3 rounded-lg border border-danger/30 bg-danger/10 px-3 py-2 text-xs text-danger"
        >
          {error}
        </p>
      )}
    </>
  );

  const installed = useMemo(
    () =>
      (entries.data ?? [])
        .filter(isInstalled)
        .sort((a, b) => a.game.title.localeCompare(b.game.title)),
    [entries.data],
  );

  if (entries.isLoading) {
    return (
      <>
        {header}
        <Empty icon="installed" title="Loading…">Reading your library.</Empty>
      </>
    );
  }

  if (installed.length === 0) {
    return (
      <>
        {header}
        <Empty icon="installed" title="Nothing installed yet">
          Download a game and install it from the Downloads tab.
        </Empty>
      </>
    );
  }

  return (
    <>
    {header}
    <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
      <div className="flex flex-col gap-2">
        {installed.map((entry) => (
          <InstalledRow key={entry.game.id} entry={entry} />
        ))}
      </div>
    </div>
    </>
  );
}

function InstalledRow({ entry }: { entry: LibraryEntry }) {
  const [expanded, setExpanded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmUninstall, setConfirmUninstall] = useState(false);
  const queryClient = useQueryClient();

  // Awaited, so a failed launch shows an error instead of looking like nothing happened.
  async function play() {
    setError(null);
    try {
      await backend.launch(entry.game.id);
    } catch (e) {
      setError(messageOf(e));
    }
  }

  async function uninstall(options: { runUninstaller: boolean; uninstaller: string | null }) {
    setConfirmUninstall(false);
    setError(null);
    try {
      // The uninstaller (chosen in the dialog) runs first to clear registry entries and shortcuts.
      await backend.uninstall(entry.game.id, options.runUninstaller, options.uninstaller);
      await queryClient.invalidateQueries({ queryKey: ["entries"] });
    } catch (e) {
      setError(messageOf(e));
    }
  }
  const running = entry.state.kind === "running";
  const path = entry.state.kind === "installed" ? entry.state.path : null;
  const executable = entry.state.kind === "installed" ? entry.state.executable : null;
  const setups = entry.state.kind === "installed" ? (entry.state.setupCandidates ?? []) : [];
  // Offered dismissably, not done: deleting the leftover archive silently would be presumptuous.
  const [dismissedCleanup, setDismissedCleanup] = useState(false);
  const stagingSetups =
    entry.state.kind === "installed" ? (entry.state.stagingSetups ?? []) : [];
  const stagingPresent = entry.state.kind === "installed" && Boolean(entry.state.stagingPresent);
  const canReclaim =
    entry.state.kind === "installed" && (entry.archivePresent || stagingPresent) && !dismissedCleanup;
  // Nothing to play until setup has run.
  const needsSetup = setups.length > 0 && !executable;

  return (
    <article className="rounded-xl border border-default-200 bg-content1">
      <div className="flex items-center gap-4 p-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h3 className="truncate text-sm font-medium text-foreground">{entry.game.title}</h3>
            {running && (
              <span className="flex shrink-0 items-center gap-1 rounded bg-success/15 px-1.5 py-0.5 text-[10px] font-medium text-success">
                <span className="h-1 w-1 animate-pulse rounded-full bg-success" />
                Running
              </span>
            )}
          </div>
          <p className="truncate text-xs text-foreground/45" title={path ?? undefined}>
            {executable ?? path ?? ""}
          </p>
        </div>

        <span className="shrink-0 text-xs text-foreground/50">
          {formatPlaytime(entry.minutesPlayed)}
        </span>

        <button
          type="button"
          disabled={running || !executable}
          title={executable ? undefined : "Choose an executable first"}
          onClick={() => void play()}
          className="flex shrink-0 items-center gap-1.5 rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-primary-600 disabled:cursor-not-allowed disabled:opacity-40"
        >
          <Icon name="play" className="h-3 w-3" filled />
          {running ? "Playing" : "Play"}
        </button>

        <button
          type="button"
          aria-label={expanded ? "Hide options" : "Show options"}
          onClick={() => setExpanded((e) => !e)}
          className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-foreground/45 transition-colors hover:bg-default-100 hover:text-foreground"
        >
          <Icon
            name="chevron"
            className={`h-4 w-4 transition-transform ${expanded ? "rotate-90" : ""}`}
          />
        </button>
      </div>

      {needsSetup && (
        <div className="flex flex-wrap items-center gap-2 border-t border-warning/30 bg-warning/10 px-3 py-2">
          <span className="text-xs text-warning-600">
            This download contained a setup program. Run it to finish installing.
          </span>
          {setups.map((setup) => (
            <button
              key={setup}
              type="button"
              onClick={() => void backend.runSetup(entry.game.id, setup)}
              className="rounded-lg bg-warning px-2.5 py-1 text-xs font-medium text-white transition-colors hover:bg-warning-600"
            >
              Run {setup}
            </button>
          ))}
        </div>
      )}

      {stagingSetups.length > 0 && (
        <div className="flex flex-wrap items-center gap-2 border-t border-default-200/60 bg-primary/5 px-3 py-2">
          <span className="text-xs text-foreground/60">
            The download also contains{" "}
            {stagingSetups.length === 1 ? "another installer" : "more installers"}, usually DLC or
            an expansion. Run it into the same folder to add it.
          </span>
          {stagingSetups.map((setup) => (
            <button
              key={setup}
              type="button"
              onClick={() => void backend.runSetup(entry.game.id, setup)}
              className="rounded-lg border border-default-200 px-2.5 py-1 font-mono text-[11px] text-foreground/70 transition-colors hover:bg-default-100"
              title={setup}
            >
              Run {setup.length > 40 ? `${setup.slice(0, 37)}…` : setup}
            </button>
          ))}
        </div>
      )}

      {canReclaim && (
        <div className="flex flex-wrap items-center gap-2 border-t border-default-200/60 bg-default-100/50 px-3 py-2">
          <span className="text-xs text-foreground/60">
            This game is installed, so its{" "}
            {stagingPresent ? "unpacked files are" : "download is"} no longer needed.
          </span>
          <button
            type="button"
            onClick={async () => {
              try {
                if (stagingPresent) await backend.deleteStaging(entry.game.id);
                if (entry.archivePresent) await backend.deleteDownload(entry.game.id);
                await queryClient.invalidateQueries({ queryKey: ["entries"] });
              } catch (e) {
                setError(messageOf(e));
              }
            }}
            className="rounded-lg border border-default-200 px-2.5 py-1 text-xs text-foreground/70 transition-colors hover:bg-default-100"
          >
            Free up space
          </button>
          <button
            type="button"
            onClick={() => setDismissedCleanup(true)}
            className="text-xs text-foreground/40 underline-offset-2 hover:text-foreground/70 hover:underline"
          >
            Keep it
          </button>
        </div>
      )}

      {entry.state.kind === "preparing" && (
        <p className="border-t border-primary/30 bg-primary/10 px-3 py-2 text-xs text-foreground/70">
          {entry.state.message}
        </p>
      )}

      {error && (
        <p
          role="alert"
          className="border-t border-danger/30 bg-danger/10 px-3 py-2 text-xs text-danger"
        >
          {error}
        </p>
      )}

      {expanded && (
        <Options
          entry={entry}
          installDir={path}
          current={executable ?? null}
          setups={setups}
          onUninstall={() => setConfirmUninstall(true)}
        />
      )}

      {confirmUninstall && (
        <UninstallDialog
          title={entry.game.title}
          gameId={entry.game.id}
          installDir={path}
          onConfirm={(options) => void uninstall(options)}
          onCancel={() => setConfirmUninstall(false)}
        />
      )}
    </article>
  );
}

function Options({
  entry,
  installDir,
  current,
  setups,
  onUninstall,
}: {
  entry: LibraryEntry;
  installDir: string | null;
  current: string | null;
  setups: string[];
  onUninstall: () => void;
}) {
  const gameId = entry.game.id;
  // Its own error line: the page-level error is far from this expanded row.
  const [folderError, setFolderError] = useState<string | null>(null);
  const executables = useQuery({
    queryKey: ["executables", gameId],
    queryFn: () => backend.listExecutables(gameId),
  });

  return (
    <div className="flex flex-col gap-3 border-t border-default-200/60 px-3 py-3">
      <div>
        <p className="mb-1 text-[11px] text-foreground/45">Launch executable</p>
        {executables.data && executables.data.length > 0 ? (
          <select
            value={current ?? ""}
            onChange={(e) => void backend.setExecutable(gameId, e.target.value)}
            className="w-full rounded-lg border border-default-200 bg-content2 px-2 py-1.5 font-mono text-[11px] outline-none focus:border-primary"
          >
            <option value="" disabled>
              Choose an executable…
            </option>
            {executables.data.map((exe) => (
              <option key={exe} value={exe}>
                {exe}
              </option>
            ))}
          </select>
        ) : (
          <p className="text-[11px] text-foreground/40">
            {executables.isLoading ? "Scanning…" : "No launchable file found in this folder."}
          </p>
        )}
      </div>

      {installDir && (
        <div>
          <p className="mb-1 text-[11px] text-foreground/45">Installed at</p>
          <code className="block break-all text-[11px] text-foreground/60">{installDir}</code>
        </div>
      )}

      <GameOptions gameId={gameId} />

      <ShortcutOptions gameId={gameId} />

      {setups.length > 0 && (
        <div>
          <p className="mb-1 text-[11px] text-foreground/45">Setup programs</p>
          <div className="flex flex-wrap gap-1.5">
            {setups.map((setup) => (
              <button
                key={setup}
                type="button"
                onClick={() => void backend.runSetup(gameId, setup)}
                className="rounded-lg border border-default-200 px-2.5 py-1 font-mono text-[11px] text-foreground/70 transition-colors hover:bg-default-100"
              >
                {setup}
              </button>
            ))}
          </div>
        </div>
      )}

      {folderError && (
        <p role="alert" className="text-[11px] leading-relaxed text-danger">
          {folderError}
        </p>
      )}

      <div className="flex gap-2">
        <button
          type="button"
          onClick={async () => {
            // Awaited, so a rejection surfaces instead of the click doing nothing silently.
            setFolderError(null);
            try {
              await backend.openFolder(gameId, "installations");
            } catch (e) {
              setFolderError(messageOf(e));
            }
          }}
          className="rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/70 transition-colors hover:bg-default-100"
        >
          Open folder
        </button>
        <button
          type="button"
          onClick={onUninstall}
          className="ml-auto rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/60 transition-colors hover:border-danger/40 hover:bg-danger/10 hover:text-danger"
        >
          Uninstall
        </button>
      </div>
    </div>
  );
}

