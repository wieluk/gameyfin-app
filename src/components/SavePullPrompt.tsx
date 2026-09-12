import { useState } from "react";

import { backend } from "@/lib/backend";
import { formatBytes, formatRelative } from "@/lib/format";
import { keys, useInvalidate } from "@/lib/queries";
import { useAction } from "@/lib/useAction";
import { useTauriEvent } from "@/lib/useTauriEvent";
import { Alert } from "@/components/Alert";
import type { SavePullOffer } from "@/bindings/SavePullOffer";
import { Modal, ModalFooter, ModalHeader } from "./Modal";

/**
 * Asks, before a game is played here for the first time, whether to download the saves
 * that already exist for it.
 *
 * Restoring writes over whatever save files are on disk, and a game played before sync was
 * set up has saves no backup has captured. The launch waits on this answer, which is why
 * the dialog starts the game itself.
 */
export function SavePullPrompt() {
  const invalidate = useInvalidate();
  const action = useAction();
  // Queued: two launches in a row would otherwise leave the first game waiting forever.
  const [queue, setQueue] = useState<SavePullOffer[]>([]);
  const offer = queue[0];

  useTauriEvent<SavePullOffer>("save-pull-offer", (next) => {
    setQueue((current) =>
      current.some((o) => o.gameId === next.gameId) ? current : [...current, next],
    );
  });

  if (!offer) return null;

  function dismiss() {
    if (!action.busy) setQueue((current) => current.slice(1));
  }

  async function answer(download: boolean) {
    const answered = await action.run(async () => {
      await backend.answerSavePullOffer(offer.gameId, download);
      await invalidate(keys.saveOverviewAll);
      // The launch stopped to ask, so it has to be started again either way.
      await backend.launch(offer.gameId);
      return true;
    });
    if (answered) setQueue((current) => current.slice(1));
  }

  const from = offer.device ? `from ${offer.device}` : "from another PC";

  return (
    // Escape means "not now": the game does not start, and the offer returns next time.
    <Modal label={`Saves found for ${offer.title}`} role="alertdialog" onDismiss={dismiss}>
      <ModalHeader
        title={`Saves already exist for ${offer.title}`}
        description={
          <>
            There is a save {from}, {formatRelative(offer.remoteAt)}
            {offer.sizeBytes > 0 ? `, ${formatBytes(offer.sizeBytes)}` : ""}. This PC has never
            synced this game. Downloading replaces any save files already on it, so keep yours
            if you have played this game here before.
          </>
        }
      />
      {action.error && (
        <div className="px-5 pb-2">
          <Alert>{action.error}</Alert>
        </div>
      )}

      <ModalFooter>
        <button
          type="button"
          disabled={action.busy}
          onClick={dismiss}
          className="rounded-lg px-3 py-1.5 text-xs text-foreground/60 hover:bg-default-100 disabled:opacity-50"
        >
          Not now
        </button>
        <button
          type="button"
          disabled={action.busy}
          onClick={() => void answer(false)}
          className="rounded-lg border border-default-200 px-3 py-1.5 text-xs font-medium hover:bg-default-100 disabled:opacity-50"
        >
          Keep mine and play
        </button>
        <button
          type="button"
          disabled={action.busy}
          onClick={() => void answer(true)}
          className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-white hover:bg-primary-600 disabled:opacity-50"
        >
          {action.busy ? "Working…" : "Download and play"}
        </button>
      </ModalFooter>
    </Modal>
  );
}
