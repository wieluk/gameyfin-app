import { useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { FolderActions } from "@/components/FolderActions";
import { LaunchOptions, SetupOptions } from "@/components/GameOptions";
import { Icon } from "@/components/Icon";
import { ShortcutOptions } from "@/components/ShortcutOptions";
import { UninstallDialog } from "@/components/UninstallDialog";
import { installedFiles, isInstalled } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { formatPlaytime } from "@/lib/format";
import { messageOf } from "@/lib/errors";
import type { LibraryEntry } from "@/types";
import { Empty } from "@/components/Empty";
import { keys, useEntries } from "@/lib/queries";
import { useRescanOnOpen } from "@/lib/rescan";
import { BUTTON, PANEL_BODY } from "@/lib/ui";

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
    <div className={PANEL_BODY}>
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

  // Awaited, so a failure shows an error instead of looking like nothing happened.
  async function run(work: () => Promise<unknown>) {
    setError(null);
    try {
      await work();
    } catch (e) {
      setError(messageOf(e));
    }
  }

  const play = () => run(() => backend.launch(entry.game.id));
  const runSetup = (setup: string) => run(() => backend.runSetup(entry.game.id, setup));

  function uninstall(options: { runUninstaller: boolean; uninstaller: string | null }) {
    setConfirmUninstall(false);
    // The uninstaller (chosen in the dialog) runs first to clear registry entries and shortcuts.
    return run(async () => {
      await backend.uninstall(entry.game.id, options.runUninstaller, options.uninstaller);
      await queryClient.invalidateQueries({ queryKey: keys.entries });
    });
  }

  const stop = () =>
    run(async () => {
      await backend.stopGame(entry.game.id);
      await queryClient.invalidateQueries({ queryKey: keys.entries });
    });
  const running = entry.state.kind === "running";
  // While it runs, the row says what is running and what it is running through.
  const nowRunning = entry.state.kind === "running" ? entry.state : null;
  // Read through the helper, so a game that failed to start keeps its Play button and its
  // executable picker instead of greying out.
  const { path, executable, setupCandidates: setups, stagingSetups, stagingPresent } =
    installedFiles(entry.state);
  // Offered dismissably, not done: deleting the leftover archive silently would be presumptuous.
  const [dismissedCleanup, setDismissedCleanup] = useState(false);
  // A prefix being set up, an installer running, or the last attempt's failure.
  const busy = entry.state.kind === "installed" ? (entry.state.busy ?? null) : null;
  const working = busy !== null && busy.kind !== "failed";
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
            {nowRunning?.runtime && (
              <span
                className="truncate text-[10px] text-foreground/45"
                title={nowRunning.runtime}
              >
                {nowRunning.runtime}
              </span>
            )}
          </div>
          <p className="truncate text-xs text-foreground/45" title={path ?? undefined}>
            {nowRunning?.executable ?? executable ?? path ?? ""}
          </p>
        </div>

        <span className="shrink-0 text-xs text-foreground/50">
          {formatPlaytime(entry.minutesPlayed)}
        </span>

        {running ? (
          // A game that will not close leaves the row saying "Playing" forever, with
          // nothing to press. Stopping ends its Wine prefix, the same way an install does.
          <button
            type="button"
            onClick={() => void stop()}
            className="flex shrink-0 items-center gap-1.5 rounded-lg border border-default-200 px-3 py-1.5 text-xs font-medium text-foreground/70 transition-colors hover:border-danger/40 hover:bg-danger/10 hover:text-danger"
          >
            <Icon name="close" className="h-3 w-3" />
            Stop
          </button>
        ) : (
          <button
            type="button"
            disabled={!executable || working}
            title={
              working
                ? "Busy with setup"
                : executable
                  ? undefined
                  : "Choose an executable first"
            }
            onClick={() => void play()}
            className="flex shrink-0 items-center gap-1.5 rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-primary-600 disabled:cursor-not-allowed disabled:opacity-40"
          >
            <Icon name="play" className="h-3 w-3" filled />
            Play
          </button>
        )}

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
              onClick={() => void runSetup(setup)}
              className="rounded-lg bg-warning px-2.5 py-1 text-xs font-medium text-white transition-colors hover:bg-warning-600"
            >
              Run {setup}
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

      {busy && (
        <p
          className={`border-t px-3 py-2 text-xs ${
            busy.kind === "failed"
              ? "border-danger/30 bg-danger/10 text-danger"
              : "border-primary/30 bg-primary/10 text-foreground/70"
          }`}
        >
          {busy.kind === "installing" ? "Running a setup program…" : busy.message}
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
          onRunSetup={runSetup}
          stagingSetups={stagingSetups}
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
  onRunSetup,
  stagingSetups,
  onUninstall,
}: {
  entry: LibraryEntry;
  installDir: string | null;
  current: string | null;
  setups: string[];
  onRunSetup: (setup: string) => void;
  stagingSetups: string[];
  onUninstall: () => void;
}) {
  const gameId = entry.game.id;
  // Installers from the game's folder and from the leftover download, which `runSetup`
  // looks through in that order. The same name can appear in both.
  const extraSetups = [...new Set([...setups, ...stagingSetups])];
  const queryClient = useQueryClient();
  // Its own error line: the page-level error is far from this expanded row.
  const [folderError, setFolderError] = useState<string | null>(null);
  const executables = useQuery({
    queryKey: ["executables", gameId],
    queryFn: () => backend.listExecutables(gameId),
  });

  async function chooseExecutable(relative: string) {
    setFolderError(null);
    try {
      await backend.setExecutable(gameId, relative);
      // The select is controlled by the stored value, so it snaps back to the old one
      // until the catalogue is refetched.
      await queryClient.invalidateQueries({ queryKey: ["entries"] });
    } catch (e) {
      setFolderError(messageOf(e));
    }
  }

  return (
    <div className="flex flex-col gap-3 border-t border-default-200/60 px-3 py-3">
      <div>
        <p className="mb-1 text-[11px] text-foreground/45">Launch executable</p>
        {executables.data && executables.data.length > 0 ? (
          <select
            value={current ?? ""}
            onChange={(e) => void chooseExecutable(e.target.value)}
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

      <LaunchOptions gameId={gameId} />

      <ShortcutOptions gameId={gameId} />

      {/* Folded away: by the time a game is installed, running a setup again is the DLC
          and repair case, not the usual one. */}
      {extraSetups.length > 0 && (
        <details className="rounded-lg border border-default-200/60 px-2.5 py-1.5">
          <summary className="cursor-pointer text-[11px] text-foreground/45">
            Setup programs ({extraSetups.length})
          </summary>
          <p className="mt-2 text-[11px] leading-relaxed text-foreground/45">
            Installers that came with this game, usually DLC or an expansion. Run one to add
            it to the same folder.
          </p>
          <div className="mt-2 flex flex-wrap gap-1.5">
            {extraSetups.map((setup) => (
              <button
                key={setup}
                type="button"
                onClick={() => onRunSetup(setup)}
                title={setup}
                className="rounded-lg border border-default-200 px-2.5 py-1 font-mono text-[11px] text-foreground/70 transition-colors hover:bg-default-100"
              >
                Run {setup.length > 40 ? `${setup.slice(0, 37)}…` : setup}
              </button>
            ))}
          </div>
          <div className="mt-2">
            <SetupOptions gameId={gameId} hint="Flags for the setup programs above." />
          </div>
        </details>
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
              await backend.openGameFolder(gameId, "installations");
            } catch (e) {
              setFolderError(messageOf(e));
            }
          }}
          className={BUTTON}
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

