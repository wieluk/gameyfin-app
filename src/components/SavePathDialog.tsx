import { useEffect, useState } from "react";
import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { Modal } from "./Modal";
import type { SaveLocations } from "@/bindings/SaveLocations";

/** Where browsing starts: the game's own Windows folder if it has one, else this PC's. */
function saveHome(locations: SaveLocations | null): string | undefined {
  return (
    locations?.detected[0] ??
    locations?.prefixHome ??
    locations?.prefixDriveC ??
    locations?.installDir ??
    locations?.home ??
    undefined
  );
}

/** A folder picker for a path field, so nobody has to type one out. */
function Browse({
  startIn,
  onPick,
}: {
  startIn?: string;
  onPick: (path: string) => void;
}) {
  return (
    <button
      type="button"
      onClick={async () => {
        const picked = await backend.pickFolder(startIn);
        if (picked) onPick(picked);
      }}
      className="shrink-0 rounded-lg border border-default-200 px-2.5 py-1.5 text-[11px] text-foreground/70 hover:bg-default-100"
    >
      Browse…
    </button>
  );
}

/** One rewrite: where the save actually is, and what it should be recorded as. */
interface Mapping {
  source: string;
  target: string;
}

/** Correcting where a game's saves live when the helper looks in the wrong place. */
export function SavePathDialog({
  gameId,
  gameTitle,
  onClose,
  onSaved,
}: {
  gameId: number;
  gameTitle: string;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [locations, setLocations] = useState<SaveLocations | null>(null);
  const [folders, setFolders] = useState<string[]>([""]);
  const [mappings, setMappings] = useState<Mapping[]>([{ source: "", target: "" }]);
  const [translate, setTranslate] = useState(false);
  const [busy, setBusy] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Read here, so the dialog opens on what is set and saving cannot wipe it.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const current = await backend.savePaths(gameId);
        if (cancelled) return;
        setFolders(current.customPaths.length > 0 ? current.customPaths : [""]);
        setMappings(
          current.redirects.length > 0
            ? current.redirects.map(([source, target]) => ({ source, target }))
            : [{ source: "", target: "" }],
        );
        setTranslate(current.crossOs);
        setLoaded(true);
      } catch (e) {
        if (!cancelled) setError(messageOf(e));
      }
      try {
        // Asked with a scan: this is the one place worth waiting a moment to be told
        // where the game's saves actually turned out to be.
        const found = await backend.saveLocations(gameId, true);
        if (!cancelled) setLocations(found);
      } catch {
        // Only the browse buttons' starting folder; typing a path still works.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [gameId]);

  function update(index: number, field: keyof Mapping, value: string) {
    setMappings((current) =>
      current.map((row, i) => (i === index ? { ...row, [field]: value } : row)),
    );
  }

  async function save() {
    setBusy(true);
    setError(null);
    try {
      // Half-filled rows are a mistake, not an instruction.
      const complete = mappings
        .map(({ source, target }) => [source.trim(), target.trim()] as [string, string])
        .filter(([source, target]) => source && target);
      // Blank rows are the empty starting row, not an instruction to clear the folders.
      const paths = folders.map((folder) => folder.trim()).filter(Boolean);
      await backend.setSaveMapping(gameId, translate, complete, paths);
      onSaved();
      onClose();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal label={`Save locations for ${gameTitle}`} size="lg" onDismiss={onClose}>
      <div className="px-5 py-4">
        <h2 className="mb-1 text-sm font-semibold">Save locations for {gameTitle}</h2>
        <p className="text-xs leading-relaxed text-foreground/60">
          Use this when the helper does not know this game, or looks in the wrong place.
        </p>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-5">
        <p className="mb-1 text-xs font-medium">Save folders</p>
        <p className="mb-2 text-[11px] leading-relaxed text-foreground/50">
          Where this game keeps its saves on this PC. Naming one is what makes a game the
          database has never heard of backupable at all. A game it does know keeps
          everything it already found, and gains these as well.
        </p>
        {folders.map((folder, index) => (
          <div key={index} className="mb-2 flex items-center gap-2">
            <input
              className="min-w-0 flex-1 rounded-lg border border-default-200 bg-content2 px-3 py-1.5 text-xs"
              placeholder="/home/you/.local/share/ExampleGame"
              value={folder}
              onChange={(e) =>
                setFolders((c) => c.map((v, i) => (i === index ? e.target.value : v)))
              }
            />
            <Browse
              startIn={folder || saveHome(locations)}
              onPick={(picked) =>
                setFolders((c) => c.map((v, i) => (i === index ? picked : v)))
              }
            />
            <button
              type="button"
              aria-label="Remove this folder"
              onClick={() => setFolders((c) => c.filter((_, i) => i !== index))}
              className="shrink-0 rounded-lg px-2 py-1 text-xs text-foreground/50 hover:bg-default-100"
            >
              &times;
            </button>
          </div>
        ))}
        {locations?.prefixHome && (
          <p className="mb-2 text-[11px] leading-relaxed text-foreground/50">
            Browse starts inside this game&rsquo;s Windows folder. A folder picked in there
            is stored by its place in it, not by its path on this PC, so the save still
            lands in the right place on a Windows machine.
          </p>
        )}
        <button
          type="button"
          onClick={() => setFolders((c) => [...c, ""])}
          className="mb-4 rounded-lg bg-default-100 px-3 py-1.5 text-xs hover:bg-default-200"
        >
          Add a folder
        </button>

        <p className="mb-1 border-t border-default-200 pt-3 text-xs font-medium">
          Path corrections
        </p>
        <p className="mb-2 text-[11px] leading-relaxed text-foreground/50">
          Only needed when a save has to travel between machines that keep it in different
          places. The first box is the folder here, the second the name it is stored
          under, which has to match on every machine you sync with.
        </p>
        {mappings.map((row, index) => (
          <div key={index} className="mb-2 flex items-center gap-2">
            <input
              className="min-w-0 flex-1 rounded-lg border border-default-200 bg-content2 px-3 py-1.5 text-xs"
              placeholder={"C:\\Users\\you\\Documents\\My Games\\Example"}
              value={row.source}
              onChange={(e) => update(index, "source", e.target.value)}
            />
            <span className="shrink-0 text-xs text-foreground/40">to</span>
            <input
              className="min-w-0 flex-1 rounded-lg border border-default-200 bg-content2 px-3 py-1.5 text-xs"
              placeholder="/gameyfin/home/Example"
              value={row.target}
              onChange={(e) => update(index, "target", e.target.value)}
            />
            <button
              type="button"
              aria-label="Remove this mapping"
              onClick={() => setMappings((c) => c.filter((_, i) => i !== index))}
              className="shrink-0 rounded-lg px-2 py-1 text-xs text-foreground/50 hover:bg-default-100"
            >
              &times;
            </button>
          </div>
        ))}
        <button
          type="button"
          onClick={() => setMappings((c) => [...c, { source: "", target: "" }])}
          className="mb-3 rounded-lg bg-default-100 px-3 py-1.5 text-xs hover:bg-default-200"
        >
          Add another
        </button>

        <label className="mb-3 flex cursor-pointer items-start gap-2">
          <input
            type="checkbox"
            className="mt-1"
            checked={translate}
            onChange={(e) => setTranslate(e.target.checked)}
          />
          <span>
            <span className="text-xs">Translate between Windows and Linux paths</span>
            <span className="block text-[11px] text-foreground/50">
              Only for a save that crossed between a Windows machine and a native Linux
              build. A Windows game played here through Proton needs nothing: its saves
              already travel by way of its Windows folder. Turning this on replaces that
              with Ludusavi&rsquo;s own translation, which is best effort and does not
              carry registry settings.
            </span>
          </span>
        </label>
      </div>

      {error && <p className="px-5 py-2 text-xs text-danger">{error}</p>}

      <div className="flex justify-end gap-2 border-t border-default-200/60 px-5 py-3">
        <button
          type="button"
          onClick={onClose}
          disabled={busy}
          className="rounded-lg px-3 py-1.5 text-xs text-foreground/70 hover:bg-default-100 disabled:opacity-50"
        >
          Cancel
        </button>
        <button
          type="button"
          onClick={save}
          // Saving before the current paths arrive would write the empty starting state
          // over them.
          disabled={busy || !loaded}
          className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-white hover:bg-primary/90 disabled:opacity-50"
        >
          {busy ? "Saving..." : "Save"}
        </button>
      </div>
    </Modal>
  );
}
