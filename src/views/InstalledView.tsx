import { useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Alert } from "@/components/Alert";
import { FolderActions } from "@/components/FolderActions";
import { LaunchOptions, SetupOptions } from "@/components/GameOptions";
import { PrefixOptions } from "@/components/PrefixOptions";
import { SetupPicker } from "@/components/SetupList";
import { ShortcutOptions } from "@/components/ShortcutOptions";
import { TransferProgress } from "@/components/TransferProgress";
import { UninstallDialog } from "@/components/UninstallDialog";
import { UntrackedFolders } from "@/components/UntrackedFolders";
import { Button, FormField, IconButton, Select, Switch, ViewHeader } from "@/components/ui";
import { installedFiles, isInstalled } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { formatInstallProgress, formatPlaytime } from "@/lib/format";
import { messageOf } from "@/lib/errors";
import { arranged, defaultSelection, remaining, runOrder, toggled } from "@/lib/setups";
import type { LibraryEntry } from "@/types";
import { Empty } from "@/components/Empty";
import { keys, useEntries } from "@/lib/queries";
import { useRescanOnOpen } from "@/lib/rescan";
import { usePreloaded } from "@/lib/usePreloaded";
import { PANEL_BODY } from "@/lib/ui";

/** Games actually present on this machine and ready to play. */
export function InstalledView() {
  const entries = useEntries();
  const [error, setError] = useState<string | null>(null);

  useRescanOnOpen();

  const header = (
    <ViewHeader
      title="Installed"
      help="installed"
      error={error}
      actions={<FolderActions folder="installations" onError={setError} />}
    />
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
        <div className="px-6 pb-5">
          <UntrackedFolders folder="installations" />
        </div>
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
      <UntrackedFolders folder="installations" />
    </div>
    </>
  );
}

function InstalledRow({ entry }: { entry: LibraryEntry }) {
  const [expanded, setExpanded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The uninstaller is looked for first, so the dialog opens knowing what it will run.
  const uninstallPrompt = usePreloaded((gameId: number) => backend.findUninstaller(gameId));
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
  const runSetup = (setup: string) => run(() => backend.installSetups(entry.game.id, [setup]));

  function uninstall(options: { runUninstaller: boolean; uninstaller: string | null }) {
    uninstallPrompt.close();
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
      <div className="flex items-center gap-3 p-3">
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
          <Button variant="destructive" icon="close" onClick={() => void stop()}>
            Stop
          </Button>
        ) : (
          <Button
            variant="primary"
            icon="play"
            iconFilled
            disabled={!executable || working}
            title={
              working
                ? "Busy with setup"
                : executable
                  ? undefined
                  : "Choose an executable first"
            }
            onClick={() => void play()}
          >
            Play
          </Button>
        )}

        <IconButton
          icon="chevron"
          size="sm"
          label={expanded ? "Hide options" : "Show options"}
          aria-expanded={expanded}
          iconClassName={`h-4 w-4 transition-transform ${expanded ? "rotate-90" : ""}`}
          onClick={() => setExpanded((e) => !e)}
        />
      </div>

      {needsSetup && (
        <div className="flex flex-wrap items-center gap-2 border-t border-warning/30 bg-warning/10 px-3 py-2">
          <span className="text-xs text-warning-600">
            This download contained a setup program. Run it to finish installing.
          </span>
          {setups.map((setup) => (
            <Button key={setup} size="sm" variant="primary" onClick={() => void runSetup(setup)}>
              Run {setup}
            </Button>
          ))}
        </div>
      )}

      {canReclaim && (
        <div className="flex flex-wrap items-center gap-2 border-t border-default-200/60 bg-default-100/50 px-3 py-2">
          <span className="text-xs text-foreground/60">
            This game is installed, so its{" "}
            {stagingPresent ? "unpacked files are" : "download is"} no longer needed.
          </span>
          <Button
            size="sm"
            onClick={async () => {
              try {
                if (stagingPresent) await backend.deleteStaging(entry.game.id);
                if (entry.archivePresent) await backend.deleteDownload(entry.game.id);
                await queryClient.invalidateQueries({ queryKey: ["entries"] });
              } catch (e) {
                setError(messageOf(e));
              }
            }}
          >
            Free up space
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setDismissedCleanup(true)}>
            Keep it
          </Button>
        </div>
      )}

      {busy && (
        <div
          className={`border-t px-3 py-2 text-xs ${
            busy.kind === "failed"
              ? "border-danger/30 bg-danger/10 text-danger"
              : "border-primary/30 bg-primary/10 text-foreground/70"
          }`}
        >
          <p>
            {busy.kind === "installing"
              ? busy.step
                ? `Installing ${busy.step}…`
                : "Running a setup program…"
              : busy.message}
          </p>
          {busy.kind === "preparing" && busy.progress && (
            <div className="mt-2">
              <TransferProgress {...busy.progress} />
            </div>
          )}
          {busy.kind === "installing" && busy.progress && (
            <div className="mt-2">
              <TransferProgress {...busy.progress} label={formatInstallProgress(busy.progress)} />
            </div>
          )}
        </div>
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
          stagingSetups={stagingSetups}
          onUninstall={() => void run(() => uninstallPrompt.open(entry.game.id))}
        />
      )}

      {uninstallPrompt.opened && (
        <UninstallDialog
          title={entry.game.title}
          detected={uninstallPrompt.opened.data}
          installDir={path}
          onConfirm={(options) => void uninstall(options)}
          onCancel={uninstallPrompt.close}
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
  stagingSetups,
  onUninstall,
}: {
  entry: LibraryEntry;
  installDir: string | null;
  current: string | null;
  setups: string[];
  stagingSetups: string[];
  onUninstall: () => void;
}) {
  const gameId = entry.game.id;
  // Installers from the leftover download and the game's folder. The same name can be in both.
  const extraSetups = [...new Set([...setups, ...stagingSetups])];
  // A leftover download can hold programs no name gave away, listed under the setups.
  const downloadLeft = installedFiles(entry.state).stagingPresent;
  const queryClient = useQueryClient();
  // Its own error line: the page-level error is far from this expanded row.
  const [folderError, setFolderError] = useState<string | null>(null);
  // What the user just picked, until the catalogue answers with it: the select is
  // controlled by the stored value and would otherwise snap back to the old one.
  const [picked, setPicked] = useState<string | null>(null);
  const executables = useQuery({
    queryKey: ["executables", gameId],
    queryFn: () => backend.listExecutables(gameId),
  });

  const chosen = picked ?? current;
  const listed = executables.data ?? [];
  // The list holds only files that are there, so a choice missing from it is a file that
  // has been deleted or moved since it was chosen.
  const missing = Boolean(chosen) && listed.length > 0 && !listed.includes(chosen as string);

  async function chooseExecutable(relative: string) {
    setFolderError(null);
    setPicked(relative);
    try {
      await backend.setExecutable(gameId, relative);
      await queryClient.invalidateQueries({ queryKey: ["entries"] });
      await queryClient.invalidateQueries({ queryKey: ["executables", gameId] });
    } catch (e) {
      setFolderError(messageOf(e));
    } finally {
      // The stored value is authoritative again: a browsed file was picked by its full
      // path and is stored relative to the game's folder.
      setPicked(null);
    }
  }

  /** Any file in the game's folder, for a launcher the scan did not think was one. */
  async function browseForExecutable() {
    setFolderError(null);
    try {
      const file = await backend.pickFile(installDir ?? undefined);
      if (file) await chooseExecutable(file);
    } catch (e) {
      setFolderError(messageOf(e));
    }
  }

  return (
    <div className="flex flex-col gap-3 border-t border-default-200/60 px-3 py-3">
      <FormField label="Launch executable" htmlFor={`executable-${gameId}`}>
        <div className="flex gap-2">
          <Select
            id={`executable-${gameId}`}
            mono
            value={chosen ?? ""}
            onChange={(e) => void chooseExecutable(e.target.value)}
            disabled={executables.isLoading}
            className="flex-1"
          >
            <option value="" disabled>
              {executables.isLoading ? "Scanning…" : "Choose an executable…"}
            </option>
            {/* Kept in the list so the box is not blank about a file that is gone. */}
            {missing && <option value={chosen as string}>{chosen} (missing)</option>}
            {listed.map((exe) => (
              <option key={exe} value={exe}>
                {exe}
              </option>
            ))}
          </Select>
          <Button
            onClick={() => void browseForExecutable()}
            title="Pick any file in this game's folder"
          >
            Browse…
          </Button>
        </div>
        {missing ? (
          <p className="text-[11px] text-warning-600">
            That file is no longer in the game's folder. Choose another, or browse for it.
          </p>
        ) : (
          !executables.isLoading &&
          listed.length === 0 && (
            <p className="text-[11px] text-foreground/40">
              Nothing launchable was found here. Browse to point at the file yourself.
            </p>
          )
        )}
      </FormField>

      {installDir && (
        <div>
          <p className="mb-1 text-xs text-foreground/55">Installed at</p>
          <code className="block break-all text-[11px] text-foreground/60">{installDir}</code>
        </div>
      )}

      <LaunchOptions gameId={gameId} />

      <PrefixOptions gameId={gameId} title={entry.game.title} />

      <ShortcutOptions gameId={gameId} />

      {(extraSetups.length > 0 || downloadLeft) && <InstalledSetups gameId={gameId} />}

      {folderError && <Alert inline>{folderError}</Alert>}

      <div className="flex gap-2">
        <Button
          icon="folder"
          onClick={async () => {
            // Awaited, so a rejection surfaces instead of the click doing nothing silently.
            setFolderError(null);
            try {
              await backend.openGameFolder(gameId, "installations");
            } catch (e) {
              setFolderError(messageOf(e));
            }
          }}
        >
          Open folder
        </Button>
        <Button variant="destructive" className="ml-auto" onClick={onUninstall}>
          Uninstall
        </Button>
      </div>
    </div>
  );
}

/**
 * Folded away: once a game is installed, running a setup again is the DLC and patch case.
 * Loaded when the row opens, so the list is ready by the time it is unfolded.
 */
function InstalledSetups({ gameId }: { gameId: number }) {
  const queryClient = useQueryClient();
  const setups = useQuery({
    queryKey: keys.setups(gameId),
    queryFn: () => backend.listSetups(gameId),
  });
  const [picked, setPicked] = useState<string[] | null>(null);
  // Off until asked for: a wizard shows what it installs and where.
  const [silent, setSilent] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Null until rearranged, so the plan's order stands.
  const [order, setOrder] = useState<string[] | null>(null);
  const list = arranged(setups.data ?? [], order);
  const selected = picked ?? defaultSelection(list);
  const left = remaining(list).length;
  const recognised = list.filter((s) => s.matched);
  const toggle = (path: string, on: boolean) => setPicked(toggled(list, selected, path, on));

  async function install() {
    setError(null);
    try {
      await backend.installSetups(gameId, runOrder(list, selected), silent);
      setPicked(null);
      await queryClient.invalidateQueries({ queryKey: keys.entries });
      await queryClient.invalidateQueries({ queryKey: keys.setups(gameId) });
    } catch (e) {
      setError(messageOf(e));
    }
  }

  if (setups.isLoading || list.length === 0) return null;

  return (
    <details className="rounded-lg border border-default-200/60 px-2.5 py-1.5">
      <summary className="cursor-pointer text-[11px] text-foreground/45">
        Setup programs{recognised.length > 0 ? ` (${recognised.length})` : ""}
        {left > 0 ? `, ${left} not installed` : ""}
      </summary>
      <p className="mt-2 text-[11px] leading-relaxed text-foreground/45">
        Installers that came with this game, usually DLC or patches. They install into the same
        folder, in the order shown.
      </p>
      <div className="mt-2">
        <SetupPicker setups={list} selected={selected} onToggle={toggle} onOrder={setOrder} />
      </div>
      <div className="mt-2 flex flex-wrap items-center gap-3">
        <Switch
          label="Install silently"
          checked={silent}
          onChange={setSilent}
          disabled={!list.some((s) => s.silent)}
        />
        <Button
          size="sm"
          variant="primary"
          className="ml-auto"
          disabled={selected.length === 0}
          onClick={() => void install()}
        >
          {selected.length > 1 ? `Install ${selected.length} setups` : "Install"}
        </Button>
      </div>
      {error && (
        <Alert inline className="mt-2">
          {error}
        </Alert>
      )}
      <div className="mt-2">
        <SetupOptions gameId={gameId} hint="Flags for the setup programs above." />
      </div>
    </details>
  );
}
