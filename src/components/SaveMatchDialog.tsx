import { useEffect, useState } from "react";
import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { Modal } from "./Modal";

/** Choosing which save-manifest game a library game is, when the automatic search will not guess. */
export function SaveMatchDialog({
  gameId,
  gameTitle,
  candidates,
  onClose,
  onChosen,
}: {
  gameId: number;
  gameTitle: string;
  /** Near misses the automatic search already found. */
  candidates: string[];
  onClose: () => void;
  onChosen: () => void;
}) {

  const [query, setQuery] = useState(gameTitle);
  const [results, setResults] = useState<string[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Searching shells out to the backup helper, so it waits for a pause in typing.
  useEffect(() => {
    const needle = query.trim();
    if (!needle) {
      setResults(null);
      return;
    }
    let cancelled = false;
    const timer = setTimeout(async () => {
      setSearching(true);
      try {
        const found = await backend.searchSaveTitles(gameId, needle);
        if (!cancelled) setResults(found);
      } catch (e) {
        if (!cancelled) setError(messageOf(e));
      } finally {
        if (!cancelled) setSearching(false);
      }
    }, 400);

    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [query, gameId]);

  async function choose(title: string) {
    setBusy(true);
    setError(null);
    try {
      await backend.setSaveTitle(gameId, title);
      onChosen();
      onClose();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(false);
    }
  }

  // Before a search returns, the near misses already found are better than nothing.
  const shown = results ?? candidates;

  return (
    <Modal label={`Choose the save data for ${gameTitle}`} onDismiss={onClose}>
      <div className="px-5 py-4">
        <h2 className="mb-1 text-sm font-semibold">Which game is this?</h2>
        <p className="text-xs leading-relaxed text-foreground/60">
          The save helper could not tell which game <strong>{gameTitle}</strong> is. Pick it
          from the list, or search for the name it is published under.
        </p>
        <input
          autoFocus
          className="mt-3 w-full rounded-lg border border-default-200 bg-content2 px-3 py-1.5 text-xs"
          placeholder="Search by name"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto border-t border-default-200/60">
        {searching && <p className="px-5 py-3 text-xs text-foreground/50">Searching...</p>}
        {!searching && shown.length === 0 && (
          <p className="px-5 py-3 text-xs text-foreground/50">
            Nothing matches that name. The game may not be in the save database at all, in
            which case you can point at its save folder yourself instead.
          </p>
        )}
        {shown.map((title) => (
          <button
            key={title}
            type="button"
            disabled={busy}
            onClick={() => choose(title)}
            className="block w-full px-5 py-2 text-left text-xs hover:bg-default-100 disabled:opacity-50"
          >
            {title}
          </button>
        ))}
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
      </div>
    </Modal>
  );
}
