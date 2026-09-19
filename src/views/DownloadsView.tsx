import { useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Alert } from "@/components/Alert";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { DownloadProvider } from "@/components/DownloadProvider";
import { FolderActions } from "@/components/FolderActions";
import { InstallDialog } from "@/components/InstallDialog";
import { SpeedLimit } from "@/components/SpeedLimit";
import { TransferProgress } from "@/components/TransferProgress";
import { UntrackedFolders } from "@/components/UntrackedFolders";
import { Button, ViewHeader } from "@/components/ui";
import { isInDownloads, needsChooser, primaryAction } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { formatBytes, formatEta, formatInstallProgress, formatSpeed } from "@/lib/format";
import { messageOf } from "@/lib/errors";
import type { LibraryEntry } from "@/types";
import { Empty } from "@/components/Empty";
import { keys, useEntries } from "@/lib/queries";
import { useRescanOnOpen } from "@/lib/rescan";
import { usePreloaded } from "@/lib/usePreloaded";
import { PANEL_BODY } from "@/lib/ui";

/** Transfers in progress, and finished downloads awaiting the separate decision to install. */
export function DownloadsView() {
  const entries = useEntries();
  // Inspecting the download first, so the dialog opens complete.
  const install = usePreloaded((entry: LibraryEntry) => backend.installOptions(entry.game.id));
  const [deleting, setDeleting] = useState<LibraryEntry | null>(null);
  const [error, setError] = useState<string | null>(null);
  const queryClient = useQueryClient();

  useRescanOnOpen();

  async function confirmDelete() {
    const target = deleting;
    setDeleting(null);
    if (!target) return;
    try {
      await backend.deleteDownload(target.game.id);
      await queryClient.invalidateQueries({ queryKey: ["entries"] });
    } catch (e) {
      setError(messageOf(e));
    }
  }

  const rows = useMemo(
    () => (entries.data ?? []).filter(isInDownloads),
    [entries.data],
  );

  const header = (
    <ViewHeader
      title="Downloads"
      help="downloads"
      error={error}
      actions={
        <>
          <DownloadProvider onError={setError} />
          <SpeedLimit />
          <FolderActions folder="downloads" onError={setError} />
        </>
      }
    />
  );

  if (entries.isLoading) {
    return (
      <>
        {header}
        <Empty icon="download" title="Loading…">Reading your library.</Empty>
      </>
    );
  }

  if (rows.length === 0) {
    return (
      <>
        {header}
        <Empty icon="download" title="No downloads">
          Downloads appear here and stay until you install them.
        </Empty>
        <div className="px-6 pb-5">
          <UntrackedFolders folder="downloads" />
        </div>
      </>
    );
  }

  // Awaited throughout: a dropped rejection leaves the click doing nothing, saying nothing
  // and logging nothing.
  async function run(work: () => Promise<unknown>) {
    setError(null);
    try {
      await work();
    } catch (e) {
      setError(messageOf(e));
    }
  }

  // The download folder specifically, even for a game installed elsewhere too.
  const openFolder = (gameId: number) => run(() => backend.openGameFolder(gameId, "downloads"));
  // Windows will not let this app elevate a program itself, so the retry goes back out
  // through the shell and Windows shows its own consent dialog.
  const runAsAdministrator = (gameId: number) =>
    run(async () => {
      await backend.runSetupElevated(gameId);
      await queryClient.invalidateQueries({ queryKey: keys.entries });
    });
  const cancelDownload = (entry: LibraryEntry) => run(() => backend.cancelDownload(entry.game.id));
  const stopInstall = (entry: LibraryEntry) => run(() => backend.stopGame(entry.game.id));

  return (
    <>
    {header}
    <div className={PANEL_BODY}>
      <div className="flex flex-col gap-3">
        {rows.map((entry) => (
          <DownloadRow
            key={entry.game.id}
            entry={entry}
            onInstall={(target) => void run(() => install.open(target))}
            opening={install.loading?.game.id === entry.game.id}
            onDelete={setDeleting}
            onCancel={cancelDownload}
            onStopInstall={stopInstall}
            onOpenFolder={openFolder}
            onElevate={runAsAdministrator}
            onRun={(work) => void run(work)}
          />
        ))}
      </div>
      <UntrackedFolders folder="downloads" />

      {install.opened && (
        <InstallDialog
          entry={install.opened.target}
          plan={install.opened.data}
          onClose={install.close}
        />
      )}

      {deleting && (
        <ConfirmDialog
          title={`Delete the download for ${deleting.game.title}?`}
          body={
            <>
              The download folder goes, with the archive and anything unpacked from it.
              Installed games stay, and you can download again later.
            </>
          }
          confirmLabel="Delete download"
          onConfirm={() => void confirmDelete()}
          onCancel={() => setDeleting(null)}
        />
      )}
    </div>
    </>
  );
}

function DownloadRow({
  entry,
  onInstall,
  onDelete,
  onCancel,
  onStopInstall,
  onOpenFolder,
  onElevate,
  onRun,
  opening,
}: {
  entry: LibraryEntry;
  /** The install dialog is loading for this row. */
  opening: boolean;
  onInstall: (entry: LibraryEntry) => void;
  onDelete: (entry: LibraryEntry) => void;
  onCancel: (entry: LibraryEntry) => void;
  onStopInstall: (entry: LibraryEntry) => void;
  onOpenFolder: (gameId: number) => void;
  onElevate: (gameId: number) => void;
  onRun: (work: () => Promise<unknown>) => void;
}) {
  const { game, state } = entry;
  const action = primaryAction(state);
  // A plain "Retry install" would fail in exactly the same way, so when Windows asked
  // for administrator rights that is the button offered instead.
  const needsElevation = state.kind === "failed" && Boolean(state.elevationRequired);

  return (
    <article className="rounded-xl border border-default-200 bg-content1 p-4">
      <div className="mb-2 flex items-baseline justify-between gap-4">
        <h3 className="truncate text-sm font-medium text-foreground">{game.title}</h3>
        <span className="shrink-0 text-xs text-foreground/50">{statusText(entry)}</span>
      </div>

      {(state.kind === "downloading" ||
        state.kind === "extracting" ||
        state.kind === "installing") && (
        <>
          <div className="h-1.5 overflow-hidden rounded-full bg-default-200">
            <div
              className={`h-full rounded-full transition-[width] ${
                state.kind === "downloading" ? "bg-primary" : "bg-warning"
              } ${progressPercent(entry) === undefined ? "animate-pulse" : ""}`}
              style={{ width: `${progressPercent(entry) ?? 100}%` }}
            />
          </div>
          <div className="mt-2 flex items-center justify-between gap-3 text-[11px] text-foreground/45">
            <span className="min-w-0 truncate">
              {state.kind === "downloading"
                ? formatSpeed(state.bytesPerSecond)
                : state.kind === "extracting"
                  ? `${Math.round(state.percent)}%`
                  : [state.step, formatInstallProgress(state.progress)].filter(Boolean).join(", ")}
            </span>
            <div className="flex items-center gap-3">
              <span>
                {state.kind === "downloading"
                  ? (formatEta(state.receivedBytes, state.totalBytes, state.bytesPerSecond) ?? "")
                  : state.kind === "installing" && state.progress?.bytesPerSecond
                    ? formatSpeed(state.progress.bytesPerSecond)
                    : ""}
              </span>
              {/* Extraction always finishes, so it cannot be stopped; an installer can wedge,
                  so it can, and "Retry install" starts it over. */}
              {state.kind === "downloading" && (
                <Button size="sm" variant="destructive" onClick={() => onCancel(entry)}>
                  Cancel
                </Button>
              )}
              {state.kind === "installing" && (
                <Button size="sm" variant="destructive" onClick={() => onStopInstall(entry)}>
                  Stop
                </Button>
              )}
            </div>
          </div>
        </>
      )}

      {state.kind === "preparing" && (
        <div className="rounded-lg border border-primary/30 bg-primary/10 px-3 py-2 text-xs text-foreground/70">
          <p>{state.message}</p>
          {state.progress && (
            <div className="mt-2">
              <TransferProgress {...state.progress} />
            </div>
          )}
        </div>
      )}

      {state.kind === "failed" && (
        <Alert>
          {state.message}
          {needsElevation && (
            <span className="mt-1 block text-foreground/60">
              Windows asks for permission before an installer may change the system.
              Choosing this shows Windows&rsquo; own confirmation.
            </span>
          )}
        </Alert>
      )}

      {(state.kind === "downloaded" ||
        state.kind === "extracted" ||
        state.kind === "failed") && (
        <div className="mt-3 flex gap-2">
          {needsElevation ? (
            <Button variant="primary" icon="installed" onClick={() => onElevate(game.id)}>
              Run as administrator
            </Button>
          ) : (
            <Button
              variant="primary"
              icon={action.icon}
              disabled={opening}
              onClick={() => {
                // Extracting and installing both involve a choice; retrying a download
                // does not.
                if (needsChooser(state)) onInstall(entry);
                else onRun(() => action.run(game.id));
              }}
            >
              {opening ? "Inspecting…" : action.label}
            </Button>
          )}
          <Button icon="folder" onClick={() => onOpenFolder(game.id)}>
            Open folder
          </Button>
          <Button variant="destructive" className="ml-auto" onClick={() => onDelete(entry)}>
            Delete
          </Button>
        </div>
      )}
    </article>
  );
}

/** Undefined when the total is unknown, so the bar pulses. */
function progressPercent(entry: LibraryEntry): number | undefined {
  const { state } = entry;
  if (state.kind === "downloading") {
    return state.totalBytes > 0 ? (state.receivedBytes / state.totalBytes) * 100 : 0;
  }
  if (state.kind === "extracting") return state.percent;
  const progress = state.kind === "installing" ? state.progress : undefined;
  return progress && progress.totalBytes > 0
    ? (progress.receivedBytes / progress.totalBytes) * 100
    : undefined;
}

function statusText(entry: LibraryEntry): string {
  const { state } = entry;
  switch (state.kind) {
    case "downloading":
      return `${formatBytes(state.receivedBytes)} of ${formatBytes(state.totalBytes)}`;
    case "downloaded":
      return `Ready to extract (${formatBytes(state.bytes)})`;
    case "extracting":
      return "Extracting";
    case "extracted":
      return state.setupCandidates.length > 1
        ? `Unpacked, ${state.setupCandidates.length} setups found`
        : state.setupCandidates.length > 0
          ? "Unpacked, setup found"
          : "Unpacked";
    case "installing":
      return "Installing";
    case "preparing":
      return "Preparing";
    case "failed":
      switch (state.stage) {
        case "install":
          return "Install failed";
        case "extract":
          return "Extract failed";
        default:
          return "Download failed";
      }
    default:
      return "";
  }
}
