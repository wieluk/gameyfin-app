import { useState } from "react";

import { SaveConflictDialog } from "@/components/SaveConflictDialog";
import { backend } from "@/lib/backend";
import { keys, useInvalidate } from "@/lib/queries";
import { useAction } from "@/lib/useAction";
import { useTauriEvent } from "@/lib/useTauriEvent";
import type { SaveConflictPrompt as Prompt } from "@/bindings/SaveConflictPrompt";
import type { ConflictChoice } from "@/types";

/**
 * Settles a conflict where the automatic sync ran into it. A launch waits here and starts the
 * game once answered; "Not now" leaves it unstarted, and the question comes back next time.
 */
export function SaveConflictPrompt() {
  const invalidate = useInvalidate();
  const action = useAction();
  // Queued: two games can meet a conflict before the first is answered.
  const [queue, setQueue] = useState<Prompt[]>([]);
  const prompt = queue[0];

  useTauriEvent<Prompt>("save-conflict", (next) => {
    setQueue((current) =>
      current.some((p) => p.gameId === next.gameId) ? current : [...current, next],
    );
  });

  if (!prompt) return null;
  const launching = prompt.moment === "launch";

  function next() {
    action.setError(null);
    setQueue((current) => current.slice(1));
  }

  async function choose(choice: ConflictChoice) {
    const done = await action.run(async () => {
      await backend.resolveSaveConflict(prompt.gameId, choice);
      await invalidate(keys.saveOverviewAll);
      if (launching) await backend.launch(prompt.gameId);
      return true;
    });
    if (done) next();
  }

  return (
    <SaveConflictDialog
      title={prompt.title}
      localAt={prompt.localAt}
      remote={prompt.remote}
      busy={action.busy}
      error={action.error}
      cancelLabel={launching ? "Not now" : "Decide later"}
      onChoose={(choice) => void choose(choice)}
      onCancel={() => {
        if (!action.busy) next();
      }}
    />
  );
}
