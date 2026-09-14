import { useEffect, useState } from "react";

import { Alert } from "@/components/Alert";
import { Modal } from "@/components/Modal";
import { Button, Checkbox, TextInput } from "@/components/ui";
import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { formatBytes } from "@/lib/format";
import type { SaveFind } from "@/bindings/SaveFind";
import type { Game } from "@/types";

/**
 * Saves the helper finds on this PC, installed by Gameyfin or not. Nothing is backed up until a
 * row is ticked, and a find with no library game must be matched to one by hand.
 */
export function ScanThisPcDialog({
  games,
  onClose,
  onDone,
}: {
  /** The library, for matching a find to a game by hand. */
  games: Game[];
  onClose: () => void;
  onDone: () => void;
}) {
  const [finds, setFinds] = useState<SaveFind[] | null>(null);
  const [chosen, setChosen] = useState<ReadonlySet<string>>(new Set());
  const [matching, setMatching] = useState<SaveFind | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [progress, setProgress] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const found = await backend.scanThisPc();
        if (!cancelled) setFinds(found);
      } catch (e) {
        if (!cancelled) setError(messageOf(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  function toggle(title: string) {
    setChosen((current) => {
      const next = new Set(current);
      if (next.has(title)) next.delete(title);
      else next.add(title);
      return next;
    });
  }

  /** Backs the ticked finds up, one at a time: each one runs the helper. */
  async function backUpChosen() {
    const wanted = (finds ?? []).filter(
      (find) => chosen.has(find.ludusaviTitle) && find.gameId !== null,
    );
    setBusy(true);
    setError(null);
    let done = 0;
    let failed = false;
    for (const find of wanted) {
      setProgress(
        `Backing up ${find.gameTitle ?? find.ludusaviTitle} (${++done} of ${wanted.length})…`,
      );
      try {
        // The title first: it is what the backup is filed under, and a find is the only
        // place some of these games are named at all.
        await backend.setSaveTitle(find.gameId as number, find.ludusaviTitle);
        await backend.backupSaves(find.gameId as number, false);
      } catch (e) {
        setError(`${find.gameTitle ?? find.ludusaviTitle}: ${messageOf(e)}`);
        failed = true;
        break;
      }
    }
    setProgress(null);
    setBusy(false);
    onDone();
    // Left open on a failure: the message names which game, and closing would take it away.
    if (!failed) onClose();
  }

  const actionable = (finds ?? []).filter((find) => chosen.has(find.ludusaviTitle)).length;

  return (
    <Modal label="Saves on this PC" size="2xl" onDismiss={busy ? () => {} : onClose}>
      <div className="px-5 py-4">
        <h2 className="mb-1 text-sm font-semibold text-foreground">Saves on this PC</h2>
        <p className="text-xs leading-relaxed text-foreground/60">
          Everything the save database recognises here, including games Gameyfin never
          installed. Tick what is worth keeping and it is backed up to wherever your saves
          go.
        </p>

        {finds === null && !error && (
          <p className="mt-4 text-xs text-foreground/50">
            Looking through this PC. On a large drive this takes a few minutes.
          </p>
        )}

        {finds !== null && finds.length === 0 && (
          <p className="mt-4 text-xs text-foreground/50">
            Nothing was found. The database may not know these games, in which case a
            game&rsquo;s own row under Saves can be pointed at its folder by hand.
          </p>
        )}

        {finds !== null && finds.length > 0 && (
          <ul className="mt-3 flex max-h-[45vh] flex-col gap-1 overflow-y-auto">
            {finds.map((find) => (
              <li
                key={find.ludusaviTitle}
                className="flex items-center gap-3 rounded-lg border border-default-200/60 px-2.5 py-2"
              >
                <Checkbox
                  aria-label={`Back up ${find.ludusaviTitle}`}
                  checked={chosen.has(find.ludusaviTitle)}
                  disabled={find.gameId === null || busy}
                  onChange={() => toggle(find.ludusaviTitle)}
                />
                <div className="min-w-0 flex-1">
                  <p className="truncate text-xs text-foreground">{find.ludusaviTitle}</p>
                  <p className="truncate text-[11px] text-foreground/45">
                    {find.files} {find.files === 1 ? "file" : "files"},{" "}
                    {formatBytes(find.bytes)}
                    {find.folder ? ` in ${find.folder}` : ""}
                  </p>
                  {find.gameId !== null && find.matched !== "recorded" && (
                    <p className="truncate text-[11px] text-foreground/45">
                      Matched to {find.gameTitle} in your library.
                    </p>
                  )}
                  {find.alreadyBackedUp && (
                    <p className="truncate text-[11px] text-foreground/45">
                      Already backed up; ticking it uploads what is here now.
                    </p>
                  )}
                </div>
                {find.gameId === null && (
                  <Button size="sm" disabled={busy} onClick={() => setMatching(find)}>
                    Match to a game
                  </Button>
                )}
              </li>
            ))}
          </ul>
        )}

        {progress && (
          <p role="status" className="mt-3 text-[11px] text-foreground/60">
            {progress}
          </p>
        )}
        {error && <Alert className="mt-3">{error}</Alert>}
      </div>

      <div className="flex justify-end gap-2 border-t border-default-200/60 px-5 py-3">
        <Button variant="ghost" onClick={onClose} disabled={busy}>
          Close
        </Button>
        <Button
          variant="primary"
          onClick={() => void backUpChosen()}
          disabled={busy || actionable === 0}
        >
          {busy ? "Backing up…" : `Back up ${actionable || ""}`.trim()}
        </Button>
      </div>

      {matching && (
        <MatchToGame
          find={matching}
          games={games}
          onClose={() => setMatching(null)}
          onChosen={(game) => {
            setFinds((current) =>
              (current ?? []).map((find) =>
                find.ludusaviTitle === matching.ludusaviTitle
                  ? { ...find, gameId: game.id, gameTitle: game.title, matched: "title" }
                  : find,
              ),
            );
            setChosen((current) => new Set(current).add(matching.ludusaviTitle));
            setMatching(null);
          }}
        />
      )}
    </Modal>
  );
}

/** Picks the library game a find belongs to. The mirror of choosing a game's save data. */
function MatchToGame({
  find,
  games,
  onClose,
  onChosen,
}: {
  find: SaveFind;
  games: Game[];
  onClose: () => void;
  onChosen: (game: Game) => void;
}) {
  const [needle, setNeedle] = useState(find.ludusaviTitle);
  const shown = games
    .filter((game) => game.title.toLowerCase().includes(needle.trim().toLowerCase()))
    .slice(0, 30);

  return (
    <Modal label={`Match ${find.ludusaviTitle}`} layer={70} onDismiss={onClose}>
      <div className="px-5 py-4">
        <h2 className="mb-1 text-sm font-semibold text-foreground">
          Which game is {find.ludusaviTitle}?
        </h2>
        <p className="mb-3 text-xs leading-relaxed text-foreground/60">
          A save is stored against a game in your library, so pick the one these files
          belong to.
        </p>
        <TextInput
          value={needle}
          autoFocus
          icon="search"
          className="mb-2"
          onChange={(e) => setNeedle(e.target.value)}
          placeholder="Search your library"
        />
        <ul className="flex max-h-[40vh] flex-col gap-1 overflow-y-auto">
          {shown.map((game) => (
            <li key={game.id}>
              <button
                type="button"
                onClick={() => onChosen(game)}
                className="w-full truncate rounded-lg px-2.5 py-1.5 text-left text-xs hover:bg-default-100"
              >
                {game.title}
              </button>
            </li>
          ))}
          {shown.length === 0 && (
            <li className="px-2.5 py-1.5 text-[11px] text-foreground/45">
              Nothing in your library matches that name.
            </li>
          )}
        </ul>
      </div>
      <div className="flex justify-end border-t border-default-200/60 px-5 py-3">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
      </div>
    </Modal>
  );
}
