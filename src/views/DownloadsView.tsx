import { useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Icon } from "@/components/Icon";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { DownloadProvider } from "@/components/DownloadProvider";
import { FolderActions } from "@/components/FolderActions";
import { InstallDialog } from "@/components/InstallDialog";
import { SpeedLimit } from "@/components/SpeedLimit";
import { isInDownloads, needsChooser, primaryAction } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { formatBytes, formatEta, formatSpeed } from "@/lib/format";
import { messageOf } from "@/lib/errors";
import type { LibraryEntry } from "@/types";
import { Empty } from "@/components/Empty";
import { keys, useEntries } from "@/lib/queries";
import { useRescanOnOpen } from "@/lib/rescan";
import { BUTTON, PANEL_BODY } from "@/lib/ui";

/** Transfers in progress, and finished downloads awaiting the separate decision to install. */
export function DownloadsView() {
  const entries = useEntries();
  const [installing, setInstalling] = useState<LibraryEntry | null>(null);
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
    <div className="flex shrink-0 items-center justify-between border-b border-default-200/60 px-6 py-3">
      <h2 className="text-xs font-semibold uppercase tracking-wide text-foreground/45">
        Downloads
      </h2>
      <div className="flex items-center gap-3">
        <DownloadProvider onError={setError} />
        <SpeedLimit />
        <FolderActions folder="downloads" onError={setError} />
      </div>
    </div>
  );

  // Both actions in the header can fail with the list empty, so the error line travels
  // with the header rather than living inside the branch that renders rows.
  const errorLine = error && (
    <p
      role="alert"
      className="mx-6 mt-3 rounded-lg border border-danger/30 bg-danger/10 px-3 py-2 text-xs text-danger"
    >
      {error}
    </p>
  );

  if (entries.isLoading) {
    return (
      <>
        {header}
        {errorLine}
        <Empty icon="download" title="Loading…">Reading your library.</Empty>
      </>
    );
  }

  if (rows.length === 0) {
    return (
      <>
        {header}
        {errorLine}
        <Empty icon="download" title="No downloads">
          Downloads appear here and stay until you install them.
        </Empty>
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
            onInstall={setInstalling}
            onDelete={setDeleting}
            onCancel={cancelDownload}
            onStopInstall={stopInstall}
            onOpenFolder={openFolder}
            onElevate={runAsAdministrator}
            onRun={(work) => void run(work)}
          />
        ))}
      </div>

      {error && (
        <p
          role="alert"
          className="mt-3 rounded-lg border border-danger/30 bg-danger/10 px-3 py-2 text-xs text-danger"
        >
          {error}
        </p>
      )}

      {installing && (
        <InstallDialog entry={installing} onClose={() => setInstalling(null)} />
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
}: {
  entry: LibraryEntry;
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
              }`}
              style={{ width: `${progressPercent(entry)}%` }}
            />
          </div>
          <div className="mt-2 flex items-center justify-between gap-3 text-[11px] text-foreground/45">
            <span>
              {state.kind === "downloading"
                ? formatSpeed(state.bytesPerSecond)
                : state.kind === "extracting"
                  ? `${Math.round(state.percent)}%`
                  : "Working…"}
            </span>
            <div className="flex items-center gap-3">
              <span>
                {state.kind === "downloading"
                  ? (formatEta(state.receivedBytes, state.totalBytes, state.bytesPerSecond) ?? "")
                  : ""}
              </span>
              {/* Extraction always finishes, so it cannot be stopped; an installer can wedge,
                  so it can, and "Retry install" starts it over. */}
              {state.kind === "downloading" && (
                <button
                  type="button"
                  onClick={() => onCancel(entry)}
                  className="rounded-lg border border-default-200 px-2 py-1 text-[11px] text-foreground/60 transition-colors hover:border-danger/40 hover:bg-danger/10 hover:text-danger"
                >
                  Cancel
                </button>
              )}
              {state.kind === "installing" && (
                <button
                  type="button"
                  onClick={() => onStopInstall(entry)}
                  className="rounded-lg border border-default-200 px-2 py-1 text-[11px] text-foreground/60 transition-colors hover:border-danger/40 hover:bg-danger/10 hover:text-danger"
                >
                  Stop
                </button>
              )}
            </div>
          </div>
        </>
      )}

      {state.kind === "preparing" && (
        <p className="rounded-lg border border-primary/30 bg-primary/10 px-3 py-2 text-xs text-foreground/70">
          {state.message}
        </p>
      )}

      {state.kind === "failed" && (
        <p
          role="alert"
          className="rounded-lg border border-danger/30 bg-danger/10 px-3 py-2 text-xs text-danger"
        >
          {state.message}
          {needsElevation && (
            <span className="mt-1 block text-foreground/60">
              Windows asks for permission before an installer may change the system.
              Choosing this shows Windows&rsquo; own confirmation.
            </span>
          )}
        </p>
      )}

      {(state.kind === "downloaded" ||
        state.kind === "extracted" ||
        state.kind === "failed") && (
        <div className="mt-3 flex gap-2">
          {needsElevation ? (
            <button
              type="button"
              onClick={() => onElevate(game.id)}
              className="flex items-center gap-1.5 rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-primary-600"
            >
              <Icon name="installed" className="h-3.5 w-3.5" />
              Run as administrator
            </button>
          ) : (
            <button
              type="button"
              onClick={() => {
                // Extracting and installing both involve a choice; retrying a download
                // does not.
                if (needsChooser(state)) onInstall(entry);
                else onRun(() => action.run(game.id));
              }}
              className="flex items-center gap-1.5 rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-primary-600"
            >
              <Icon name={action.icon} className="h-3.5 w-3.5" />
              {action.label}
            </button>
          )}
          <button
            type="button"
            onClick={() => onOpenFolder(game.id)}
            className={BUTTON}
          >
            Open folder
          </button>
          <button
            type="button"
            onClick={() => onDelete(entry)}
            className="ml-auto rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/60 transition-colors hover:border-danger/40 hover:bg-danger/10 hover:text-danger"
          >
            Delete
          </button>
        </div>
      )}
    </article>
  );
}

function progressPercent(entry: LibraryEntry): number {
  const { state } = entry;
  if (state.kind === "downloading") {
    return state.totalBytes > 0 ? (state.receivedBytes / state.totalBytes) * 100 : 0;
  }
  if (state.kind === "extracting") return state.percent;
  return 0;
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
      return state.setupCandidates.length > 0 ? "Unpacked, setup found" : "Unpacked";
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

