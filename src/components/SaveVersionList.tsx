import { useQuery } from "@tanstack/react-query";

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
}: {
  gameId: number;
  busy: boolean;
  /** False for a game that is not installed here, which has nowhere to restore into. */
  canRestore?: boolean;
  onRestore: (saveId: string) => void;
}) {
  const versions = useQuery({
    queryKey: keys.saveVersions(gameId),
    queryFn: () => backend.listSaveVersions(gameId),
  });

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
  if ((versions.data ?? []).length === 0) {
    return <p className="text-[11px] text-foreground/40">Nothing stored for this game yet.</p>;
  }

  return (
    <ul className="flex flex-col gap-1">
      {(versions.data ?? []).map((version) => (
        <li key={version.id} className="flex items-center gap-3 text-[11px]">
          <span className="min-w-0 flex-1 truncate text-foreground/70">
            {formatRelative(version.createdAt)}
            {version.deviceName ? ` from ${version.deviceName}` : ""}
          </span>
          <span className="shrink-0 text-foreground/40">{platformLabel(version.platform)}</span>
          <span className="shrink-0 text-foreground/40">{formatBytes(version.sizeBytes)}</span>
          {version.locked && (
            <span className="shrink-0 text-foreground/40" title="Kept forever">
              kept
            </span>
          )}
          <button
            type="button"
            disabled={busy || !canRestore}
            title={canRestore ? undefined : "Install this game to restore its save."}
            onClick={() => onRestore(version.id)}
            className="shrink-0 rounded px-2 py-0.5 text-foreground/60 hover:bg-default-100 disabled:opacity-50"
          >
            Restore
          </button>
        </li>
      ))}
    </ul>
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
