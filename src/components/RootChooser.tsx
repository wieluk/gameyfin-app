import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Icon } from "@/components/Icon";
import { Button } from "@/components/ui";
import { backend, type LibraryRoot } from "@/lib/backend";
import { readStored, writeStored } from "@/lib/storage";
import { useDismissOnEscape } from "@/lib/useDismiss";
import { formatBytes } from "@/lib/format";

/** The configured games folders. Invalidated as `["library-roots"]`. */
export function useLibraryRoots() {
  return useQuery({ queryKey: ["library-roots"], queryFn: () => backend.listLibraryRoots() });
}

const LAST_ROOT_KEY = "gameyfin.lastRoot";

/**
 * Asks which games folder a download should go to. Shown only when there is more than
 * one; a dialog rather than a dropdown so the free space on each is visible.
 */
export function RootChooser({
  title,
  requiredBytes,
  onChoose,
  onCancel,
}: {
  title: string;
  /** The download's size, so a folder without room can be called out. */
  requiredBytes?: number;
  onChoose: (root: string) => void;
  onCancel: () => void;
}) {
  const roots = useLibraryRoots();
  const [selected, setSelected] = useState<string | null>(null);

  // Preselect where the last download went; nearly always right for this one too.
  useEffect(() => {
    if (!roots.data || roots.data.length === 0 || selected) return;
    const remembered = readStored<string | null>(LAST_ROOT_KEY, null);
    const usable = roots.data.find((root) => root.path === remembered) ?? roots.data[0];
    setSelected(usable.path);
  }, [roots.data, selected]);

  // Through the shared stack, so only the topmost dialog closes.
  useDismissOnEscape(onCancel);

  function confirm() {
    if (!selected) return;
    writeStored(LAST_ROOT_KEY, selected);
    onChoose(selected);
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm"
      role="presentation"
      onClick={onCancel}
    >
      <div
        data-nav-scope
        role="dialog"
        aria-label="Choose where to download"
        className="w-full max-w-md rounded-2xl border border-default-200 bg-content1 p-5 shadow-xl"
        onClick={(event) => event.stopPropagation()}
      >
        <h2 className="text-sm font-semibold text-foreground">Where should {title} go?</h2>
        {requiredBytes ? (
          <p className="mt-1 text-[11px] text-foreground/45">
            This download needs about {formatBytes(requiredBytes)}.
          </p>
        ) : null}

        <div className="mt-3 flex flex-col gap-1.5">
          {(roots.data ?? []).map((root) => (
            <RootOption
              key={root.path}
              root={root}
              requiredBytes={requiredBytes}
              selected={selected === root.path}
              onSelect={() => setSelected(root.path)}
            />
          ))}
        </div>

        <div className="mt-4 flex justify-end gap-2">
          <Button variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
          <Button variant="primary" onClick={confirm} disabled={!selected}>
            Download here
          </Button>
        </div>
      </div>
    </div>
  );
}

function RootOption({
  root,
  requiredBytes,
  selected,
  onSelect,
}: {
  root: LibraryRoot;
  requiredBytes?: number;
  selected: boolean;
  onSelect: () => void;
}) {
  // Called out rather than disabled: the size is only the server's estimate.
  const tooSmall =
    requiredBytes !== undefined && root.freeBytes !== null && root.freeBytes < requiredBytes;

  return (
    <button
      type="button"
      onClick={onSelect}
      aria-pressed={selected}
      className={`flex items-center gap-3 rounded-lg border px-3 py-2 text-left transition-colors ${
        selected
          ? "border-primary/50 bg-primary/10"
          : "border-default-200 hover:bg-default-100"
      }`}
    >
      <Icon
        name="folder"
        className={`h-4 w-4 shrink-0 ${selected ? "text-primary" : "text-foreground/40"}`}
      />
      <span className="min-w-0 flex-1">
        <span className="block truncate font-mono text-[11px] text-foreground" title={root.path}>
          {root.path}
        </span>
        <span className="block text-[11px] text-foreground/45">
          {!root.exists
            ? "This folder is missing"
            : root.freeBytes === null
              ? "Free space unknown"
              : `${formatBytes(root.freeBytes)} free`}
          {root.isDefault ? " (default)" : ""}
        </span>
      </span>
      {tooSmall && (
        <span className="shrink-0 rounded bg-danger/15 px-1.5 py-0.5 text-[10px] text-danger">
          Not enough room
        </span>
      )}
    </button>
  );
}
