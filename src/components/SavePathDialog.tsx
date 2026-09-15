import { useEffect, useState } from "react";
import { Alert } from "@/components/Alert";
import { Icon } from "@/components/Icon";
import { Button, IconButton, SwitchField, TextInput } from "@/components/ui";
import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import {
  browseStart,
  displayPath,
  fillStoredNames,
  storedNameFor,
  type Mapping,
} from "@/lib/savePaths";
import { HINT } from "@/lib/ui";
import { Modal, ModalFooter } from "./Modal";
import type { SaveLocations } from "@/bindings/SaveLocations";
import type { SavePathSettings } from "@/bindings/SavePathSettings";

export type LoadedSavePaths = {
  current: SavePathSettings | null;
  locations: SaveLocations | null;
  error: string | null;
};

/** What the dialog opens on. Never rejects: a failed read opens the dialog saying so. */
export async function loadSavePaths(gameId: number): Promise<LoadedSavePaths> {
  const [current, locations] = await Promise.allSettled([
    backend.savePaths(gameId),
    // Answers at once, so Browse knows where to start while the full scan runs.
    backend.saveLocations(gameId, false),
  ]);
  return {
    current: current.status === "fulfilled" ? current.value : null,
    locations: locations.status === "fulfilled" ? locations.value : null,
    error: current.status === "rejected" ? messageOf(current.reason) : null,
  };
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
    <Button
      icon="folder"
      onClick={async () => {
        const picked = await backend.pickFolder(startIn);
        if (picked) onPick(picked);
      }}
    >
      Browse…
    </Button>
  );
}

/** Correcting where a game's saves live when the helper looks in the wrong place. */
export function SavePathDialog({
  gameId,
  gameTitle,
  initial,
  onClose,
  onSaved,
}: {
  gameId: number;
  gameTitle: string;
  initial: LoadedSavePaths;
  onClose: () => void;
  onSaved: () => void;
}) {
  const current = initial.current;
  const [locations, setLocations] = useState<SaveLocations | null>(initial.locations);
  const [scanning, setScanning] = useState(true);
  const [folders, setFolders] = useState<string[]>(
    current && current.customPaths.length > 0 ? current.customPaths : [""],
  );
  const [mappings, setMappings] = useState<Mapping[]>(
    (current?.redirects ?? []).map(([source, target]) => ({ source, target })),
  );
  const [translate, setTranslate] = useState(current?.crossOs ?? false);
  // Whatever is configured must not hide behind a closed section.
  const [advanced, setAdvanced] = useState(
    Boolean(current && (current.redirects.length > 0 || current.crossOs)),
  );
  const [busy, setBusy] = useState(false);
  // Saving without what is set would wipe it.
  const loaded = current !== null;
  const [error, setError] = useState<string | null>(initial.error);

  useEffect(() => {
    let cancelled = false;
    void backend
      .saveLocations(gameId, true)
      .then(
        (found) => {
          if (!cancelled) setLocations(found);
        },
        // Only suggestions: picking or typing a path still works.
        () => {},
      )
      .finally(() => {
        if (!cancelled) setScanning(false);
      });
    return () => {
      cancelled = true;
    };
  }, [gameId]);

  function setFolder(index: number, value: string) {
    setFolders((current) => current.map((v, i) => (i === index ? value : v)));
  }

  function setMapping(index: number, patch: Partial<Mapping>) {
    setMappings((current) => current.map((row, i) => (i === index ? { ...row, ...patch } : row)));
  }

  async function save() {
    setBusy(true);
    setError(null);
    try {
      // Half-filled rows are a mistake, not an instruction.
      const redirects = fillStoredNames(mappings, gameTitle)
        .filter(({ source, target }) => source && target)
        .map(({ source, target }) => [source, target] as [string, string]);
      // Blank rows are the empty starting row, not an instruction to clear the folders.
      const paths = folders.map((folder) => folder.trim()).filter(Boolean);
      await backend.setSaveMapping(gameId, translate, redirects, paths);
      onSaved();
      onClose();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(false);
    }
  }

  const start = browseStart(locations);
  const found = locations?.detected ?? [];
  const suggestions = (locations?.suggested ?? []).filter(
    (path) => !folders.includes(path) && !found.includes(path),
  );
  // What each correction will be stored as, so a blank field shows the name it gets.
  const named = fillStoredNames(mappings, gameTitle);

  return (
    <Modal label={`Save folders for ${gameTitle}`} size="2xl" onDismiss={onClose}>
      <div className="px-5 py-4">
        <h2 className="text-sm font-semibold">Save folders for {gameTitle}</h2>
        <p className="mt-1 text-xs text-foreground/60">Where this game keeps its saves on this PC.</p>
      </div>

      <div className="max-h-[60vh] overflow-y-auto px-5 pb-4">
        {found.length > 0 && (
          <p
            className="mb-3 flex items-center gap-1.5 text-[11px] text-success-600"
            title={found.join("\n")}
          >
            <Icon name="check" className="h-3 w-3 shrink-0" />
            <span className="truncate">
              Saves already found in {displayPath(found[0], locations)}
              {found.length > 1 && ` and ${found.length - 1} more`}
            </span>
          </p>
        )}

        {suggestions.length > 0 && (
          <div className="mb-3">
            <p className={`mb-1.5 ${HINT}`}>These look like this game&rsquo;s folders:</p>
            <div className="flex flex-col gap-1.5">
              {suggestions.map((path) => (
                <div
                  key={path}
                  className="flex items-center gap-2 rounded-lg border border-dashed border-default-200 py-1 pl-3 pr-1"
                >
                  <code
                    className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground/70"
                    title={path}
                  >
                    {displayPath(path, locations)}
                  </code>
                  <Button
                    size="sm"
                    // The blank starting row is a placeholder, so the first pick replaces it.
                    onClick={() => setFolders((c) => [...c.filter((f) => f.trim()), path])}
                  >
                    Add
                  </Button>
                </div>
              ))}
            </div>
          </div>
        )}

        {folders.map((folder, index) => (
          <div key={index} className="mb-2 flex items-center gap-2">
            <TextInput
              aria-label="Save folder"
              placeholder="Pick a folder with Browse, or type a path"
              value={folder}
              onChange={(e) => setFolder(index, e.target.value)}
            />
            <Browse startIn={folder.trim() || start} onPick={(picked) => setFolder(index, picked)} />
            <IconButton
              icon="close"
              size="sm"
              label="Remove this folder"
              onClick={() => setFolders((c) => c.filter((_, i) => i !== index))}
            />
          </div>
        ))}
        <div className="flex items-center gap-3">
          <Button onClick={() => setFolders((c) => [...c, ""])}>Add a folder</Button>
          {scanning && <span className={HINT}>Looking for save folders…</span>}
        </div>

        <button
          type="button"
          aria-expanded={advanced}
          onClick={() => setAdvanced((open) => !open)}
          className="mt-4 flex w-full items-center gap-1.5 border-t border-default-200 pt-3 text-left text-xs font-medium text-foreground/70 hover:text-foreground"
        >
          <Icon
            name="chevron"
            className={`h-3.5 w-3.5 transition-transform ${advanced ? "rotate-90" : ""}`}
          />
          Different folders on other PCs
        </button>

        {advanced && (
          <div className="mt-2 flex flex-col gap-2">
            <p className={HINT}>
              Set the folder on each PC. A blank stored name comes from the title, so it matches
              everywhere.
            </p>
            {mappings.map((row, index) => (
              <div key={index} className="flex items-center gap-2">
                <TextInput
                  aria-label="Folder on this PC"
                  placeholder="Folder on this PC"
                  value={row.source}
                  onChange={(e) => setMapping(index, { source: e.target.value })}
                />
                <Browse
                  startIn={row.source.trim() || start}
                  onPick={(picked) => setMapping(index, { source: picked })}
                />
                <span className="shrink-0 text-xs text-foreground/40">stored as</span>
                <TextInput
                  aria-label="Stored name"
                  placeholder={named[index]?.target || storedNameFor(gameTitle, [])}
                  value={row.target}
                  onChange={(e) => setMapping(index, { target: e.target.value })}
                />
                <IconButton
                  icon="close"
                  size="sm"
                  label="Remove this correction"
                  onClick={() => setMappings((c) => c.filter((_, i) => i !== index))}
                />
              </div>
            ))}
            <div>
              <Button onClick={() => setMappings((c) => [...c, { source: "", target: "" }])}>
                Add a correction
              </Button>
            </div>

            {/* A prefix game always uses the portable mapping, so the switch would do nothing. */}
            {locations && !locations.savesInPrefix && (
              <div className="mt-2">
                <SwitchField
                  label="Translate Windows paths"
                  hint="For a Windows save restored into a native Linux build. Best effort, and registry settings are not carried."
                  checked={translate}
                  onChange={setTranslate}
                />
              </div>
            )}
          </div>
        )}
      </div>

      {error && <Alert className="mx-5 my-2">{error}</Alert>}

      <ModalFooter>
        <Button variant="ghost" onClick={onClose} disabled={busy}>
          Cancel
        </Button>
        <Button
          variant="primary"
          onClick={save}
          // Saving before the current paths arrive would write the empty starting state over them.
          disabled={busy || !loaded}
        >
          {busy ? "Saving…" : "Save"}
        </Button>
      </ModalFooter>
    </Modal>
  );
}
