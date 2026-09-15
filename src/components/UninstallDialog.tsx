import { useEffect, useState } from "react";

import { Alert } from "@/components/Alert";
import { Button } from "@/components/ui";
import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { Modal } from "./Modal";

/**
 * Confirmation for removing an installed game. A detected uninstaller runs first to clear
 * registry entries and shortcuts; detection is guesswork, so the user can pick one.
 */
export function UninstallDialog({
  title,
  gameId,
  installDir,
  onConfirm,
  onCancel,
}: {
  title: string;
  gameId: number;
  /** Where the game is installed. The picker opens here, and the choice must stay inside it. */
  installDir: string | null;
  onConfirm: (options: { runUninstaller: boolean; uninstaller: string | null }) => void;
  onCancel: () => void;
}) {

  const [detected, setDetected] = useState<string | null>(null);
  const [chosen, setChosen] = useState<string | null>(null);
  const [looking, setLooking] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    backend
      .findUninstaller(gameId)
      .then((found) => {
        if (live) setDetected(found);
      })
      .catch((e) => {
        if (live) setError(messageOf(e));
      })
      .finally(() => {
        if (live) setLooking(false);
      });
    return () => {
      live = false;
    };
  }, [gameId]);

  // What will actually run: a manual choice wins over detection.
  const uninstaller = chosen ?? detected;

  async function choose() {
    setError(null);
    try {
      // The game's folder: where an uninstaller is, and the only place the backend accepts one from.
      const picked = await backend.pickFile(installDir ?? undefined);
      if (picked) setChosen(picked);
    } catch (e) {
      setError(messageOf(e));
    }
  }

  return (
    <Modal label={`Uninstall ${title}`} role="alertdialog" onDismiss={onCancel}>
      <div className="px-5 py-4">
        <h2 className="mb-1 text-sm font-semibold text-foreground">Uninstall {title}?</h2>
        <p className="text-xs leading-relaxed text-foreground/60">
          The game&rsquo;s files are removed. A downloaded archive, if you still have
          one, is kept so you can reinstall without downloading again.
        </p>

        <div className="mt-3 rounded-lg border border-default-200 bg-default-100/40 px-3 py-2">
          {looking ? (
            <p className="text-xs text-foreground/50">Looking for an uninstaller…</p>
          ) : uninstaller ? (
            <>
              <p className="text-xs text-foreground/70">
                {chosen ? "Will run the program you chose:" : "Found an uninstaller:"}
              </p>
              <p className="mt-1 break-all font-mono text-[11px] text-foreground/55">
                {uninstaller}
              </p>
            </>
          ) : (
            <p className="text-xs text-foreground/70">
              No uninstaller was found, so only the files will be removed. If this game
              has one under a name Gameyfin did not recognise, choose it here.
            </p>
          )}

          <div className="mt-2 flex flex-wrap gap-2">
            <Button size="sm" onClick={() => void choose()}>
              {uninstaller ? "Choose a different one…" : "Choose uninstaller…"}
            </Button>
            {chosen && (
              <Button size="sm" variant="ghost" onClick={() => setChosen(null)}>
                Undo
              </Button>
            )}
          </div>
        </div>

        {error && (
          <Alert inline className="mt-2">
            {error}
          </Alert>
        )}
      </div>

      <div className="flex flex-wrap justify-end gap-2 border-t border-default-200/60 px-5 py-3">
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        {/* Offered only when there is something to skip: an uninstaller that hangs or
            refuses should not be the only way out of an uninstall. */}
        {uninstaller && (
          <Button onClick={() => onConfirm({ runUninstaller: false, uninstaller: null })}>
            Delete files only
          </Button>
        )}
        <Button
          variant="danger"
          icon="close"
          autoFocus
          onClick={() => onConfirm({ runUninstaller: true, uninstaller: chosen })}
        >
          {uninstaller ? "Run uninstaller" : "Uninstall"}
        </Button>
      </div>
    </Modal>
  );
}
