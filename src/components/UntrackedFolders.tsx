import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { UntrackedFolder } from "@/bindings/UntrackedFolder";
import { Alert } from "@/components/Alert";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Modal, ModalFooter, ModalHeader } from "@/components/Modal";
import { Button, TextInput } from "@/components/ui";
import { isInDownloads, isInstalled } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { formatBytes } from "@/lib/format";
import { keys, useEntries } from "@/lib/queries";
import type { LibraryEntry, LibraryFolder } from "@/types";

/** Folders in Installations or Downloads that no game claims, at the bottom of that view. */
export function UntrackedFolders({ folder }: { folder: LibraryFolder }) {
  const queryClient = useQueryClient();
  const untracked = useQuery({
    queryKey: keys.untracked,
    queryFn: () => backend.listUntrackedFolders(),
  });
  const [assigning, setAssigning] = useState<UntrackedFolder | null>(null);
  const [deleting, setDeleting] = useState<UntrackedFolder | null>(null);
  const [error, setError] = useState<string | null>(null);

  const rows = (untracked.data ?? []).filter((f) => f.folder === folder);
  if (rows.length === 0) return null;

  async function run(work: () => Promise<unknown>) {
    setError(null);
    try {
      await work();
    } catch (e) {
      setError(messageOf(e));
    }
  }

  const refresh = () =>
    Promise.all([
      queryClient.invalidateQueries({ queryKey: keys.untracked }),
      queryClient.invalidateQueries({ queryKey: keys.entries }),
    ]);

  return (
    <section className="mt-6">
      <h3 className="text-xs font-semibold uppercase tracking-wide text-foreground/40">
        Not linked to a game
      </h3>
      <p className="mb-2 mt-1 text-[11px] text-foreground/45">
        Folders in {folder === "installations" ? "Installations" : "Downloads"} that Gameyfin
        does not manage. Assign one to a game to manage it here.
      </p>
      <div className="flex flex-col gap-2">
        {rows.map((row) => (
          <div
            key={row.path}
            className="flex flex-wrap items-center gap-2 rounded-xl border border-dashed border-default-200 px-3 py-2"
          >
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm text-foreground" title={row.path}>
                {row.name}
              </p>
              <p className="truncate text-[11px] text-foreground/45">
                {formatBytes(row.bytes)}
                {row.gameId !== null ? ", its game is no longer on the server" : ""}
              </p>
            </div>
            <Button size="sm" onClick={() => setAssigning(row)}>
              Assign to a game
            </Button>
            <Button
              size="sm"
              icon="folder"
              onClick={() => void run(() => backend.openFolder(row.path))}
            >
              Open folder
            </Button>
            <Button size="sm" variant="destructive" onClick={() => setDeleting(row)}>
              Delete
            </Button>
          </div>
        ))}
      </div>
      {error && <Alert className="mt-2">{error}</Alert>}

      {assigning && (
        <AssignDialog
          folder={assigning}
          onClose={() => setAssigning(null)}
          onAssigned={() => void refresh()}
        />
      )}

      {deleting && (
        <ConfirmDialog
          title={`Delete ${deleting.name}?`}
          body={<>The folder and everything in it is removed. This cannot be undone.</>}
          confirmLabel="Delete folder"
          onConfirm={() => {
            const target = deleting;
            setDeleting(null);
            void run(async () => {
              await backend.deleteUntrackedFolder(target.path);
              await refresh();
            });
          }}
          onCancel={() => setDeleting(null)}
        />
      )}
    </section>
  );
}

const normalize = (text: string) => text.toLowerCase().replace(/[^\p{L}\p{N}]/gu, "");

/** Games the folder could be. The catalogue is already loaded, so the list shows at once. */
function candidates(entries: LibraryEntry[], folder: UntrackedFolder, query: string) {
  const needle = normalize(query);
  const free = (e: LibraryEntry) =>
    folder.folder === "installations" ? !isInstalled(e) : !isInDownloads(e);
  return entries
    .filter(free)
    .filter((e) => {
      const title = normalize(e.game.title);
      // Either way round: a folder named `Celeste.v1.4-GOG` still finds Celeste.
      return !needle || title.includes(needle) || (title.length > 2 && needle.includes(title));
    })
    .sort((a, b) => a.game.title.localeCompare(b.game.title))
    .slice(0, 50);
}

function AssignDialog({
  folder,
  onClose,
  onAssigned,
}: {
  folder: UntrackedFolder;
  onClose: () => void;
  onAssigned: () => void;
}) {
  const entries = useEntries();
  const [query, setQuery] = useState(() => folder.name.replace(/^\(\d+\)\s*/, ""));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const games = candidates(entries.data ?? [], folder, query);

  async function assign(gameId: number) {
    setBusy(true);
    setError(null);
    try {
      await backend.assignUntrackedFolder(folder.path, gameId);
      onAssigned();
      onClose();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal label={`Assign ${folder.name} to a game`} onDismiss={onClose}>
      <ModalHeader
        title={`Which game is ${folder.name}?`}
        description={
          folder.folder === "installations"
            ? "The folder is renamed after the game and becomes its install."
            : "The folder is renamed after the game and waits in Downloads to be installed."
        }
      />
      <div className="px-5 pb-3">
        <TextInput
          autoFocus
          icon="search"
          placeholder="Search your library"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>
      <div className="max-h-72 overflow-y-auto border-t border-default-200/60">
        {games.length === 0 && (
          <p className="px-5 py-3 text-xs text-foreground/50">No game in your library matches.</p>
        )}
        {games.map((entry) => (
          <button
            key={entry.game.id}
            type="button"
            disabled={busy}
            onClick={() => void assign(entry.game.id)}
            className="block w-full px-5 py-2 text-left text-xs hover:bg-default-100 disabled:opacity-50"
          >
            {entry.game.title}
          </button>
        ))}
      </div>
      {error && <Alert className="mx-5 my-2">{error}</Alert>}
      <ModalFooter>
        <Button variant="ghost" onClick={onClose} disabled={busy}>
          Cancel
        </Button>
      </ModalFooter>
    </Modal>
  );
}
