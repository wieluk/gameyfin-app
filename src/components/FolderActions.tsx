import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";

import { Button } from "@/components/ui";
import { backend, type LibraryFolder } from "@/lib/backend";
import { messageOf } from "@/lib/errors";

/** The Rescan and Open folder pair shared by the Downloads and Installed headers. */
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
      <Button onClick={() => void rescan()} disabled={rescanning}>
        {rescanning ? "Rescanning…" : "Rescan folders"}
      </Button>
      <Button
        icon="folder"
        onClick={() => void open()}
        title={
          folder === "downloads"
            ? "Open the Downloads folder"
            : "Open the Installations folder"
        }
      >
        Open folder
      </Button>
    </>
  );
}
