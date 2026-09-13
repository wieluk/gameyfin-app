import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { formatBytes, formatRelative } from "@/lib/format";
import { keys } from "@/lib/queries";
import type { SavePlatform } from "@/types";

/** Every stored version of a game's saves, newest first, shared by the game dialog and Saves tab. */
export function SaveVersionList({
  gameId,
  busy,
  canRestore = true,
  onRestore,
  onChanged,
}: {
  gameId: number;
  busy: boolean;
  /** False for a game that is not installed here, which has nowhere to restore into. */
  canRestore?: boolean;
  onRestore: (saveId: string) => void;
  /** After a delete or a keep, for a caller showing state the list does not own. */
  onChanged?: () => void;
}) {
  const queryClient = useQueryClient();
  const versions = useQuery({
    queryKey: keys.saveVersions(gameId),
    queryFn: () => backend.listSaveVersions(gameId),
  });
  // The versions a delete is waiting on a yes for: one row, or all of them.
  const [confirming, setConfirming] = useState<{ ids: string[]; all: boolean } | null>(null);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function change(action: () => Promise<void>) {
    setWorking(true);
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setWorking(false);
      setConfirming(null);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: keys.saveVersions(gameId) }),
        queryClient.invalidateQueries({ queryKey: keys.saveOverviewAll }),
      ]);
      onChanged?.();
    }
  }

  if (versions.isLoading) {
    return <p className="text-[11px] text-foreground/40">Looking…</p>;
  }
  if (versions.error) {
    return (
      <p role="alert" className="text-[11px] text-danger">
        {messageOf(versions.error)}
      </p>
    );
  }
  const list = versions.data ?? [];
  if (list.length === 0) {
    return <p className="text-[11px] text-foreground/40">Nothing stored for this game yet.</p>;
  }

  const disabled = busy || working;
  const deletingAll = confirming?.all ?? false;

  return (
    <div className="flex flex-col gap-2">
      <ul className="flex flex-col gap-1">
        {list.map((version) => (
          <li key={version.id} className="flex items-center gap-3 text-[11px]">
            <span className="min-w-0 flex-1 truncate text-foreground/70">
              {formatRelative(version.createdAt)}
              {version.deviceName ? ` from ${version.deviceName}` : ""}
            </span>
            <span className="shrink-0 text-foreground/40">{platformLabel(version.platform)}</span>
            <span className="shrink-0 text-foreground/40">{formatBytes(version.sizeBytes)}</span>

            {!deletingAll && confirming?.ids[0] === version.id ? (
              <>
                <span className="shrink-0 text-danger">Delete for good?</span>
                <button
                  type="button"
                  disabled={disabled}
                  onClick={() =>
                    void change(() => backend.deleteSaveVersions(gameId, [version.id]))
                  }
                  className="shrink-0 rounded bg-danger px-2 py-0.5 text-white hover:bg-danger/90 disabled:opacity-50"
                >
                  Delete
                </button>
                <button
                  type="button"
                  onClick={() => setConfirming(null)}
                  className="shrink-0 rounded px-2 py-0.5 text-foreground/60 hover:bg-default-100"
                >
                  Cancel
                </button>
              </>
            ) : (
              <>
                <button
                  type="button"
                  disabled={disabled}
                  aria-pressed={version.locked}
                  title={
                    version.locked
                      ? "Kept forever. Press to let it be cleaned up with old versions."
                      : "Keep this version forever"
                  }
                  onClick={() =>
                    void change(() =>
                      backend.setSaveLocked(gameId, version.id, !version.locked),
                    )
                  }
                  className={`shrink-0 rounded px-2 py-0.5 hover:bg-default-100 disabled:opacity-50 ${
                    version.locked ? "text-primary" : "text-foreground/60"
                  }`}
                >
                  {version.locked ? "Kept" : "Keep"}
                </button>
                <button
                  type="button"
                  disabled={disabled || !canRestore}
                  title={canRestore ? undefined : "Install this game to restore its save."}
                  onClick={() => onRestore(version.id)}
                  className="shrink-0 rounded px-2 py-0.5 text-foreground/60 hover:bg-default-100 disabled:opacity-50"
                >
                  Restore
                </button>
                <button
                  type="button"
                  disabled={disabled}
                  aria-label="Delete this version"
                  onClick={() => setConfirming({ ids: [version.id], all: false })}
                  className="shrink-0 rounded px-2 py-0.5 text-danger/80 hover:bg-danger/10 disabled:opacity-50"
                >
                  Delete
                </button>
              </>
            )}
          </li>
        ))}
      </ul>

      {error && (
        <p role="alert" className="text-[11px] text-danger">
          {error}
        </p>
      )}

      {list.length > 0 && (
        <div className="flex items-center justify-end gap-2 text-[11px]">
          {deletingAll ? (
            <>
              <span className="text-danger">
                Delete all {list.length === 1 ? "1 save" : `${list.length} saves`} for this game?
                They cannot be recovered.
              </span>
              <button
                type="button"
                disabled={disabled}
                onClick={() =>
                  void change(() => backend.deleteSaveVersions(gameId, confirming?.ids ?? []))
                }
                className="rounded bg-danger px-2 py-0.5 text-white hover:bg-danger/90 disabled:opacity-50"
              >
                Delete all
              </button>
              <button
                type="button"
                onClick={() => setConfirming(null)}
                className="rounded px-2 py-0.5 text-foreground/60 hover:bg-default-100"
              >
                Cancel
              </button>
            </>
          ) : (
            <button
              type="button"
              disabled={disabled}
              onClick={() => setConfirming({ ids: list.map((version) => version.id), all: true })}
              className="rounded px-2 py-0.5 text-danger/80 hover:bg-danger/10 disabled:opacity-50"
            >
              Delete all saves for this game
            </button>
          )}
        </div>
      )}
    </div>
  );
}

/** The stored tag is an enum on the wire; nobody says "PROTON" out loud. */
export function platformLabel(platform: SavePlatform | string): string {
  switch (platform) {
    case "WINDOWS":
      return "Windows";
    case "LINUX":
      return "Linux";
    case "PROTON":
      return "Windows, on Proton";
    case "MACOS":
      return "macOS";
    default:
      return "";
  }
}
