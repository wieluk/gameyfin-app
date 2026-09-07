import { useEffect, useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Icon } from "@/components/Icon";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { InstallDialog } from "@/components/InstallDialog";
import { SpeedLimit } from "@/components/SpeedLimit";
import { isInDownloads, needsChooser, primaryAction } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { formatBytes, formatEta, formatSpeed } from "@/lib/format";
import { messageOf } from "@/lib/errors";
import type { LibraryEntry } from "@/types";
import { Empty } from "@/components/Empty";
import { useEntries } from "@/lib/queries";

/**
 * Transfers in progress, and finished downloads awaiting installation.
 *
 * A completed download stays here rather than disappearing: the archive exists but the
 * game does not, and installing it is a separate decision the user makes.
 */
export function DownloadsView() {
  const entries = useEntries();
  const [rescanning, setRescanning] = useState(false);
  const [installing, setInstalling] = useState<LibraryEntry | null>(null);
  const [deleting, setDeleting] = useState<LibraryEntry | null>(null);
  const [error, setError] = useState<string | null>(null);
  const queryClient = useQueryClient();

  // Opening the tab is the moment the user expects to see what is actually on disk, so
  // adopt anything added or removed outside the app.
  useEffect(() => {
    void backend.rescanLibrary().catch(() => {});
    // Deliberately once per mount rather than on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function rescan() {
    setRescanning(true);
    setError(null);
    try {
      await backend.rescanLibrary();
      await queryClient.invalidateQueries({ queryKey: ["entries"] });
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setRescanning(false);
    }
  }

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
        <SpeedLimit />
        <button
          type="button"
          onClick={() => void rescan()}
          disabled={rescanning}
          className="rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/70 transition-colors hover:bg-default-100 disabled:opacity-50"
        >
          {rescanning ? "Rescanning…" : "Rescan folders"}
        </button>
      </div>
    </div>
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
          Downloads you start from your library appear here, and stay until you install them.
        </Empty>
      </>
    );
  }

  // `void backend.openFolder(...)` threw the promise away, so a rejection vanished: the
  // click did nothing, said nothing, and left nothing in the log to explain it.
  async function openFolder(gameId: number) {
    try {
      await backend.openFolder(gameId);
    } catch (e) {
      setError(messageOf(e));
    }
  }

  async function cancelDownload(entry: LibraryEntry) {
    try {
      await backend.cancelDownload(entry.game.id);
    } catch (e) {
      setError(messageOf(e));
    }
  }

  return (
    <>
    {header}
    <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
      <div className="flex flex-col gap-3">
        {rows.map((entry) => (
          <DownloadRow
            key={entry.game.id}
            entry={entry}
            onInstall={setInstalling}
            onDelete={setDeleting}
            onCancel={cancelDownload}
            onOpenFolder={openFolder}
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
              The downloaded file will be removed from your disk. Anything already
              installed stays where it is, and you can download the game again later.
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
  onOpenFolder,
}: {
  entry: LibraryEntry;
  onInstall: (entry: LibraryEntry) => void;
  onDelete: (entry: LibraryEntry) => void;
  onCancel: (entry: LibraryEntry) => void;
  onOpenFolder: (gameId: number) => void;
}) {
  const { game, state } = entry;
  const action = primaryAction(state);

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
                : `${Math.round(state.percent)}%`}
            </span>
            <div className="flex items-center gap-3">
              <span>
                {state.kind === "downloading"
                  ? (formatEta(state.receivedBytes, state.totalBytes, state.bytesPerSecond) ?? "")
                  : ""}
              </span>
              {/* Only downloads can be stopped: extraction and install are not resumable,
                  so interrupting them would leave a half-written game folder. */}
              {state.kind === "downloading" && (
                <button
                  type="button"
                  onClick={() => onCancel(entry)}
                  className="rounded-lg border border-default-200 px-2 py-1 text-[11px] text-foreground/60 transition-colors hover:border-danger/40 hover:bg-danger/10 hover:text-danger"
                >
                  Cancel
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
        </p>
      )}

      {(state.kind === "downloaded" ||
        state.kind === "extracted" ||
        state.kind === "failed") && (
        <div className="mt-3 flex gap-2">
          <button
            type="button"
            onClick={() => {
              // Extracting and installing both involve a choice; retrying a download
              // does not.
              if (needsChooser(state)) onInstall(entry);
              else void action.run(game.id);
            }}
            className="flex items-center gap-1.5 rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-primary-600"
          >
            <Icon name={action.icon} className="h-3.5 w-3.5" />
            {action.label}
          </button>
          <button
            type="button"
            onClick={() => onOpenFolder(game.id)}
            className="rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/70 transition-colors hover:bg-default-100"
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
  if (state.kind === "extracting" || state.kind === "installing") return state.percent;
  return 0;
}

function statusText(entry: LibraryEntry): string {
  const { state } = entry;
  switch (state.kind) {
    case "downloading":
      return `${formatBytes(state.receivedBytes)} of ${formatBytes(state.totalBytes)}`;
    case "downloaded":
      return `Ready to extract · ${formatBytes(state.bytes)}`;
    case "extracting":
      return "Extracting";
    case "extracted":
      return state.setupCandidates.length > 0 ? "Unpacked · setup found" : "Unpacked";
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

