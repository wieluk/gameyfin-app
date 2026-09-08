import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import { backend, isMockBackend, type WineProgress } from "@/lib/backend";
import { formatBytes, formatSpeed } from "@/lib/format";
import { messageOf } from "@/lib/errors";
import { isWindows } from "@/lib/platform";
import { useAppSettings } from "@/lib/queries";

/**
 * Offers the Wine download at startup, on Linux, when there is none. Without it every
 * Windows game is unplayable, and the only clue was a greyed-out install screen.
 */
export function WinePrompt() {
  const queryClient = useQueryClient();
  const settings = useAppSettings();
  const [dismissed, setDismissed] = useState(false);
  const [dontAsk, setDontAsk] = useState(false);
  const [downloading, setDownloading] = useState(false);
  const [progress, setProgress] = useState<WineProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  const asked = settings.data?.winePromptDismissed ?? true;
  // Only worth the release lookup on a platform that needs Wine, and only while unasked.
  const enabled = !isWindows && !isMockBackend && settings.isSuccess && !asked;

  const status = useQuery({
    queryKey: ["wine-status"],
    queryFn: () => backend.wineStatus(),
    staleTime: 5 * 60 * 1000,
    enabled,
  });

  useEffect(() => {
    if (!downloading) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      const off = await listen<WineProgress>("wine-progress", (event) => {
        if (!cancelled) setProgress(event.payload);
      });
      // Drop the listener if the download finished before it attached.
      if (cancelled) off();
      else unlisten = off;
    })();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [downloading]);

  // Nothing shows until the check has answered: a dialog that appears and then vanishes
  // because Wine was there all along is worse than a moment's delay.
  if (!enabled || dismissed || !status.isSuccess || status.data.installed) return null;

  const latest = status.data.latest;

  async function close() {
    setDismissed(true);
    if (!dontAsk) return;
    try {
      await backend.setWinePromptDismissed(true);
      await settings.refetch();
    } catch {
      // The dialog is already gone for this run; a failed write only means it returns.
    }
  }

  async function download() {
    setDownloading(true);
    setError(null);
    setProgress(null);
    try {
      await backend.installWine();
      await status.refetch();
      // The install options screen greys out without a runtime; tell it one exists now.
      await queryClient.invalidateQueries({ queryKey: ["install-options"] });
      setDismissed(true);
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setDownloading(false);
      setProgress(null);
    }
  }

  return (
    <div
      data-nav-scope
      className="fixed inset-0 z-[60] flex items-center justify-center bg-black/60 p-6 backdrop-blur-sm"
      role="dialog"
      aria-modal="true"
      aria-label="Download Wine"
    >
      <div className="w-full max-w-md overflow-hidden rounded-2xl border border-default-200 bg-content1 shadow-2xl">
        <div className="px-5 py-4">
          <h2 className="mb-1 text-sm font-semibold text-foreground">
            Download Wine to play Windows games?
          </h2>
          <p className="text-xs leading-relaxed text-foreground/60">
            Gameyfin keeps its own copy, so nothing is installed on your system and no
            admin rights are needed. Skip this if your library is native Linux games.
          </p>

          {downloading && (
            <div className="mt-3 flex flex-col gap-1">
              <div className="h-1.5 w-full overflow-hidden rounded-full bg-default-200">
                <div
                  className="h-full bg-primary transition-[width]"
                  style={{
                    width: progress?.totalBytes
                      ? `${Math.round((progress.receivedBytes / progress.totalBytes) * 100)}%`
                      : "0%",
                  }}
                />
              </div>
              <p className="text-[11px] text-foreground/45">
                {progress
                  ? `${formatBytes(progress.receivedBytes)} of ${formatBytes(progress.totalBytes)} at ${formatSpeed(progress.bytesPerSecond)}`
                  : "Starting download…"}
              </p>
            </div>
          )}

          {error && (
            <p role="alert" className="mt-3 text-[11px] leading-relaxed text-danger">
              {error}
            </p>
          )}

          <label className="mt-4 flex items-center gap-2 text-xs text-foreground/60">
            <input
              type="checkbox"
              checked={dontAsk}
              disabled={downloading}
              onChange={(e) => setDontAsk(e.target.checked)}
              className="h-3.5 w-3.5 accent-primary"
            />
            Don&rsquo;t ask again
          </label>
        </div>

        <div className="flex justify-end gap-2 border-t border-default-200/60 px-5 py-3">
          <button
            type="button"
            disabled={downloading}
            onClick={() => void close()}
            className="rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/70 transition-colors hover:bg-default-100 disabled:opacity-50"
          >
            Not now
          </button>
          <button
            type="button"
            autoFocus
            disabled={downloading}
            onClick={() => void download()}
            className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground transition-colors hover:bg-primary-600 disabled:opacity-50"
          >
            {downloading
              ? "Downloading…"
              : `Download Wine${latest ? ` (${formatBytes(latest.sizeBytes)})` : ""}`}
          </button>
        </div>
      </div>
    </div>
  );
}
