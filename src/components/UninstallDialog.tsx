import { useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Alert } from "@/components/Alert";
import { Button, Checkbox } from "@/components/ui";
import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { formatBytes } from "@/lib/format";
import { isWindows } from "@/lib/platform";
import { keys } from "@/lib/queries";
import { HINT } from "@/lib/ui";
import { Modal } from "./Modal";

export type UninstallOptions = {
  runUninstaller: boolean;
  uninstaller: string | null;
  deletePrefix: boolean;
};

/**
 * Confirmation for removing an installed game. A detected uninstaller runs first to clear
 * registry entries and shortcuts; detection is guesswork, so the user can pick one.
 */
export function UninstallDialog({
  gameId,
  title,
  detected,
  installDir,
  onConfirm,
  onCancel,
}: {
  gameId: number;
  title: string;
  /** Looked for before the dialog opens. */
  detected: string | null;
  /** Where the game is installed. The picker opens here, and the choice must stay inside it. */
  installDir: string | null;
  onConfirm: (options: UninstallOptions) => void;
  onCancel: () => void;
}) {
  const [chosen, setChosen] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Off by default: some Windows games keep their saves inside the prefix.
  const [deletePrefix, setDeletePrefix] = useState(false);

  const prefixes = useQuery({
    queryKey: keys.prefixes,
    queryFn: () => backend.listPrefixes(),
    enabled: !isWindows,
  });
  const prefix = prefixes.data?.find((entry) => entry.gameId === gameId);

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
          {uninstaller ? (
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

        {prefix && (
          <div className="mt-3">
            <label className="flex cursor-pointer items-center gap-2 text-xs text-foreground/80">
              <Checkbox
                checked={deletePrefix}
                onChange={(e) => setDeletePrefix(e.target.checked)}
              />
              Also delete the compatibility prefix ({formatBytes(prefix.bytes)})
            </label>
            <p className={`mt-1 ${HINT}`}>
              Anything the game saved inside the prefix goes with it, which for some Windows
              games includes save files. Kept, it is reused if you install the game again.
            </p>
          </div>
        )}

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
          <Button
            onClick={() => onConfirm({ runUninstaller: false, uninstaller: null, deletePrefix })}
          >
            Delete files only
          </Button>
        )}
        <Button
          variant="danger"
          icon="close"
          autoFocus
          onClick={() => onConfirm({ runUninstaller: true, uninstaller: chosen, deletePrefix })}
        >
          {uninstaller ? "Run uninstaller" : "Uninstall"}
        </Button>
      </div>
    </Modal>
  );
}
