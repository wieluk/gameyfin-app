import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "react-router-dom";

import { Modal } from "@/components/Modal";
import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { keys } from "@/lib/queries";
import { describe, restoredText } from "@/lib/saveState";
import { useTauriEvent } from "@/lib/useTauriEvent";
import type { SaveSyncProgress } from "@/bindings/SaveSyncProgress";

/** How long a finished sync stays on screen before it takes itself away. */
const LINGER_MS = 1600;
/** Longer after a restore, which names a folder worth having time to read. */
const RESTORED_LINGER_MS = 6000;

/**
 * What the automatic sync is doing while a game starts and after it closes. A restore that
 * failed holds the launch here until the user says what to do.
 */
export function SaveSyncStatus() {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [progress, setProgress] = useState<SaveSyncProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const closing = useRef<ReturnType<typeof setTimeout> | null>(null);

  useTauriEvent<SaveSyncProgress>("save-sync-progress", (next) => {
    if (closing.current) clearTimeout(closing.current);
    setError(null);
    setProgress(next);

    if (!isFinal(next)) return;
    void queryClient.invalidateQueries({ queryKey: keys.saveOverviewAll });
    void queryClient.invalidateQueries({ queryKey: keys.saveVersions(next.gameId) });
    // A failure stays until it is read; everything else has said what it needed to.
    if (next.phase.kind === "failed") return;
    const linger = next.phase.kind === "restored" ? RESTORED_LINGER_MS : LINGER_MS;
    closing.current = setTimeout(() => setProgress(null), linger);
  });

  useEffect(() => () => void (closing.current && clearTimeout(closing.current)), []);

  if (!progress) return null;

  const failed = progress.phase.kind === "failed";
  const held = failed && progress.blocking;

  /** Starts the game without the save, which is the user's call to make, not ours. */
  async function startAnyway(gameId: number) {
    setError(null);
    try {
      await backend.skipSaveSync(gameId);
      setProgress(null);
      await backend.launch(gameId);
    } catch (e) {
      setError(messageOf(e));
    }
  }

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
        {held && (
          <p className="mt-2 text-xs leading-relaxed text-foreground/60">
            The game has not started. Starting it now plays without that save, and nothing is
            uploaded when you stop until you decide which save to keep.
          </p>
        )}
        {error && <p className="mt-2 text-xs text-danger">{error}</p>}

        {!isFinal(progress) && (
          <div className="mt-3 h-1 w-full overflow-hidden rounded-full bg-default-200">
            <div className="h-full w-1/3 animate-pulse rounded-full bg-primary" />
          </div>
        )}
      </div>

      <div className="flex justify-end gap-2 border-t border-default-200/60 px-5 py-3">
        {held ? (
          <>
            <button
              type="button"
              onClick={() => setProgress(null)}
              className="rounded-lg px-3 py-1.5 text-xs text-foreground/70 hover:bg-default-100"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={() => {
                setProgress(null);
                navigate("/saves");
              }}
              className="rounded-lg px-3 py-1.5 text-xs text-foreground/70 hover:bg-default-100"
            >
              Open Saves
            </button>
            <button
              type="button"
              onClick={() => void startAnyway(progress.gameId)}
              className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-white hover:bg-primary/90"
            >
              Start anyway
            </button>
          </>
        ) : isFinal(progress) ? (
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
              progress.skippable ? undefined : "Stopping now would leave the save half written."
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
  return ["done", "skipped", "kept-local", "restored", "failed"].includes(progress.phase.kind);
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
      // "Backed up 2 hours ago" answers a different question when you are about to play.
      if (progress.moment === "launch" && phase.state.kind === "in-sync") {
        return "Your save is already up to date.";
      }
      return describe(phase.state).text;
    case "skipped":
      return progress.moment === "launch"
        ? "Skipped. The game starts with the save already on this PC."
        : "Not uploaded. This session started without the newer save, so choose which one to keep under Saves.";
    case "restored":
      return restoredText(phase);
    case "kept-local":
      return "Keeping this PC's save, as you chose for the newest one on the server.";
    case "failed":
      return phase.message;
  }
}
