import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";

import { Modal } from "@/components/Modal";
import { backend } from "@/lib/backend";
import { keys } from "@/lib/queries";
import { describe } from "@/lib/saveState";
import { useTauriEvent } from "@/lib/useTauriEvent";
import type { SaveSyncProgress } from "@/bindings/SaveSyncProgress";

/** How long a finished sync stays on screen before it takes itself away. */
const LINGER_MS = 1600;

/**
 * What the automatic sync is doing while a game starts and after it closes.
 *
 * Both used to happen in silence: a save that could not be restored was a log line, and a
 * failed upload was nothing at all.
 */
export function SaveSyncStatus() {
  const queryClient = useQueryClient();
  const [progress, setProgress] = useState<SaveSyncProgress | null>(null);
  const closing = useRef<ReturnType<typeof setTimeout> | null>(null);

  useTauriEvent<SaveSyncProgress>("save-sync-progress", (next) => {
    if (closing.current) clearTimeout(closing.current);
    setProgress(next);

    if (!isFinal(next)) return;
    void queryClient.invalidateQueries({ queryKey: keys.saveOverviewAll });
    void queryClient.invalidateQueries({ queryKey: keys.saveVersions(next.gameId) });
    // A failure stays until it is read; everything else has said what it needed to.
    if (next.phase.kind === "failed") return;
    closing.current = setTimeout(() => setProgress(null), LINGER_MS);
  });

  useEffect(() => () => void (closing.current && clearTimeout(closing.current)), []);

  if (!progress) return null;

  const failed = progress.phase.kind === "failed";
  return (
    <Modal
      label="Saves"
      role="alertdialog"
      size="sm"
      dismissOnBackdrop={false}
      onDismiss={() => setProgress(null)}
    >
      <div className="px-5 py-4">
        <p className="text-xs text-foreground/45">
          {progress.moment === "launch" ? "Before playing" : "After playing"}
        </p>
        <h2 className="mb-2 truncate text-sm font-semibold text-foreground">
          {progress.title}
        </h2>

        <p className={`text-xs leading-relaxed ${failed ? "text-danger" : "text-foreground/70"}`}>
          {phaseText(progress)}
        </p>

        {!isFinal(progress) && (
          <div className="mt-3 h-1 w-full overflow-hidden rounded-full bg-default-200">
            <div className="h-full w-1/3 animate-pulse rounded-full bg-primary" />
          </div>
        )}
      </div>

      <div className="flex justify-end gap-2 border-t border-default-200/60 px-5 py-3">
        {isFinal(progress) ? (
          <button
            type="button"
            onClick={() => setProgress(null)}
            className="rounded-lg px-3 py-1.5 text-xs text-foreground/70 hover:bg-default-100"
          >
            Close
          </button>
        ) : (
          <button
            type="button"
            disabled={!progress.skippable}
            title={
              progress.skippable
                ? undefined
                : "Stopping now would leave the save half written."
            }
            onClick={() => void backend.skipSaveSync(progress.gameId)}
            className="rounded-lg px-3 py-1.5 text-xs text-foreground/70 hover:bg-default-100 disabled:opacity-40"
          >
            Skip
          </button>
        )}
      </div>
    </Modal>
  );
}

function isFinal(progress: SaveSyncProgress): boolean {
  return ["done", "skipped", "nothing-to-do", "failed"].includes(progress.phase.kind);
}

/** One line saying what is happening, in the words of what it means for the save. */
function phaseText(progress: SaveSyncProgress): string {
  const phase = progress.phase;
  switch (phase.kind) {
    case "checking":
      return "Checking your saves…";
    case "downloading":
      return "Downloading the newer save…";
    case "restoring":
      return "Putting the save back…";
    case "scanning":
      return "Looking for what changed…";
    case "uploading":
      return "Uploading your save…";
    case "done":
      return describe(phase.state).text;
    case "skipped":
      return progress.moment === "launch"
        ? "Skipped. The game starts with the save already on this PC."
        : "Skipped. The backup stays on this PC until next time.";
    case "nothing-to-do":
      return "Nothing to sync.";
    case "failed":
      return phase.message;
  }
}
