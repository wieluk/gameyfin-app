import { useEffect, useState } from "react";

import { Icon } from "@/components/Icon";
import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";

/**
 * Confirmation for removing an installed game. A detected uninstaller runs first to clear
 * registry entries and shortcuts; detection is guesswork, so the dialog shows what it
 * found and lets the user point at one when it found nothing.
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
    <div
      data-nav-scope
      className="fixed inset-0 z-[60] flex items-center justify-center bg-black/60 p-6 backdrop-blur-sm"
      role="alertdialog"
      aria-modal="true"
      aria-label={`Uninstall ${title}`}
      onClick={onCancel}
    >
      <div
        className="w-full max-w-md overflow-hidden rounded-2xl border border-default-200 bg-content1 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
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
              <button
                type="button"
                onClick={() => void choose()}
                className="rounded-lg border border-default-200 px-2.5 py-1 text-[11px] text-foreground/70 transition-colors hover:bg-default-100"
              >
                {uninstaller ? "Choose a different one…" : "Choose uninstaller…"}
              </button>
              {chosen && (
                <button
                  type="button"
                  onClick={() => setChosen(null)}
                  className="rounded-lg border border-default-200 px-2.5 py-1 text-[11px] text-foreground/60 transition-colors hover:bg-default-100"
                >
                  Undo
                </button>
              )}
            </div>
          </div>

          {error && (
            <p role="alert" className="mt-2 text-[11px] leading-relaxed text-danger">
              {error}
            </p>
          )}
        </div>

        <div className="flex flex-wrap justify-end gap-2 border-t border-default-200/60 px-5 py-3">
          <button
            type="button"
            onClick={onCancel}
            className="rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/70 transition-colors hover:bg-default-100"
          >
            Cancel
          </button>
          {/* Offered only when there is something to skip: an uninstaller that hangs or
              refuses should not be the only way out of an uninstall. */}
          {uninstaller && (
            <button
              type="button"
              onClick={() => onConfirm({ runUninstaller: false, uninstaller: null })}
              className="rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/70 transition-colors hover:bg-default-100"
            >
              Delete files only
            </button>
          )}
          <button
            type="button"
            autoFocus
            onClick={() => onConfirm({ runUninstaller: true, uninstaller: chosen })}
            className="flex items-center gap-1.5 rounded-lg bg-danger px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-danger-600"
          >
            <Icon name="close" className="h-3 w-3" />
            {uninstaller ? "Run uninstaller" : "Uninstall"}
          </button>
        </div>
      </div>
    </div>
  );
}
