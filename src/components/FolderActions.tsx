import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";

import { Icon } from "@/components/Icon";
import { backend, type LibraryFolder } from "@/lib/backend";
import { messageOf } from "@/lib/errors";

/**
 * The Rescan and Open folder pair in the Downloads and Installed headers.
 *
 * Shared so the two tabs behave identically; they differ only in which folder the second
 * button reveals.
 */
export function FolderActions({
  folder,
  onError,
}: {
  folder: LibraryFolder;
  /** Where to show a failure. Both actions can fail on an unreadable or missing folder. */
  onError: (message: string | null) => void;
}) {
  const queryClient = useQueryClient();
  const [rescanning, setRescanning] = useState(false);

  async function rescan() {
    setRescanning(true);
    onError(null);
    try {
      await backend.rescanLibrary();
      await queryClient.invalidateQueries({ queryKey: ["entries"] });
    } catch (e) {
      onError(messageOf(e));
    } finally {
      setRescanning(false);
    }
  }

  async function open() {
    onError(null);
    try {
      await backend.openLibraryFolder(folder);
    } catch (e) {
      onError(messageOf(e));
    }
  }

  return (
    <>
      <button
        type="button"
        onClick={() => void rescan()}
        disabled={rescanning}
        className="rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/70 transition-colors hover:bg-default-100 disabled:opacity-50"
      >
        {rescanning ? "Rescanning…" : "Rescan folders"}
      </button>
      <button
        type="button"
        onClick={() => void open()}
        title={
          folder === "downloads"
            ? "Open the Downloads folder"
            : "Open the Installations folder"
        }
        className="flex items-center gap-1.5 rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/70 transition-colors hover:bg-default-100"
      >
        <Icon name="folder" className="h-3.5 w-3.5" />
        Open folder
      </button>
    </>
  );
}
