import { useState } from "react";

import { Alert } from "@/components/Alert";
import { platformLabel } from "@/components/SaveVersionList";
import { Button, Radio } from "@/components/ui";
import { backend } from "@/lib/backend";
import { formatBytes, formatRelative } from "@/lib/format";
import { keys, useInvalidate } from "@/lib/queries";
import { useAction } from "@/lib/useAction";
import { useTauriEvent } from "@/lib/useTauriEvent";
import type { SavePullOffer } from "@/bindings/SavePullOffer";
import { Modal, ModalFooter, ModalHeader } from "./Modal";

/**
 * Asks, on a game's first start on this PC, which stored save to play with. Always asked: the
 * game may have been played here before Gameyfin saw it. The dialog starts the game itself.
 */
export function SavePullPrompt() {
  const invalidate = useInvalidate();
  const action = useAction();
  // Queued: two launches in a row would otherwise leave the first game waiting forever.
  const [queue, setQueue] = useState<SavePullOffer[]>([]);
  // Null until the user picks, so the newest version that restores here is the default.
  const [picked, setPicked] = useState<string | null>(null);
  const offer = queue[0];

  useTauriEvent<SavePullOffer>("save-pull-offer", (next) => {
    setQueue((current) =>
      current.some((o) => o.gameId === next.gameId) ? current : [...current, next],
    );
  });

  if (!offer) return null;

  const chosen =
    picked ?? offer.versions.find((offered) => offered.restorable)?.version.id ?? null;

  function next() {
    setPicked(null);
    setQueue((current) => current.slice(1));
  }

  function dismiss() {
    if (!action.busy) next();
  }

  async function answer(saveId: string | null) {
    const answered = await action.run(async () => {
      await backend.answerSavePullOffer(offer.gameId, saveId);
      await invalidate(keys.saveOverviewAll);
      // The launch stopped to ask, so it has to be started again either way.
      await backend.launch(offer.gameId);
      return true;
    });
    if (answered) next();
  }

  return (
    // Escape means "not now": the game does not start, and the offer returns next time.
    <Modal label={`Saves found for ${offer.title}`} role="alertdialog" onDismiss={dismiss}>
      <ModalHeader
        title={`Which save for ${offer.title}?`}
        description={
          offer.localSaves ? (
            <>
              This PC already has save files for it
              {offer.localAt ? `, last changed ${formatRelative(offer.localAt)}` : ""}. Restoring
              a stored save replaces them.
            </>
          ) : (
            "First start here. Pick a stored save to play with, or start without one."
          )
        }
      />

      <ul role="radiogroup" className="flex max-h-[40vh] flex-col gap-1 overflow-y-auto px-5 pb-3">
        {offer.versions.map(({ version, restorable }) => (
          <li key={version.id}>
            <label
              className={`flex items-center gap-3 rounded-lg border px-2.5 py-2 text-xs ${
                chosen === version.id ? "border-primary bg-primary/5" : "border-default-200/60"
              } ${restorable ? "cursor-pointer" : "opacity-50"}`}
            >
              <Radio
                name="offered-save"
                checked={chosen === version.id}
                disabled={!restorable || action.busy}
                onChange={() => setPicked(version.id)}
              />
              <span className="min-w-0 flex-1 truncate">
                {formatRelative(version.createdAt)}
                {version.deviceName ? ` from ${version.deviceName}` : ""}
              </span>
              <span className="shrink-0 text-foreground/45">
                {restorable
                  ? platformLabel(version.platform)
                  : `${platformLabel(version.platform) || version.platform}, does not restore here`}
              </span>
              <span className="shrink-0 text-foreground/45">{formatBytes(version.sizeBytes)}</span>
            </label>
          </li>
        ))}
      </ul>

      {action.error && (
        <div className="px-5 pb-2">
          <Alert>{action.error}</Alert>
        </div>
      )}

      <ModalFooter>
        <Button variant="ghost" disabled={action.busy} onClick={dismiss}>
          Not now
        </Button>
        <Button disabled={action.busy} onClick={() => void answer(null)}>
          {offer.localSaves ? "Keep mine and play" : "Play without a save"}
        </Button>
        <Button
          variant="primary"
          disabled={action.busy || chosen === null}
          onClick={() => void answer(chosen)}
        >
          {action.busy ? "Working…" : "Restore and play"}
        </Button>
      </ModalFooter>
    </Modal>
  );
}
