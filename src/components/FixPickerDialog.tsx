import { useEffect, useState } from "react";

import { Alert } from "@/components/Alert";
import { Button, Select, TextInput } from "@/components/ui";
import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { HINT } from "@/lib/ui";
import type { FixMatch } from "@/bindings/FixMatch";
import type { GameFix } from "@/bindings/GameFix";
import { Modal, ModalFooter, ModalHeader } from "./Modal";

/** The stores umu files fixes under, in the order `umu.rs` lists them. */
const STORES: [string, string][] = [
  ["none", "No store"],
  ["amazon", "Amazon"],
  ["battlenet", "Battle.net"],
  ["ea", "EA"],
  ["egs", "Epic"],
  ["gog", "GOG"],
  ["humble", "Humble"],
  ["itchio", "itch.io"],
  ["steam", "Steam"],
  ["ubisoft", "Ubisoft"],
  ["zoomplatform", "ZOOM Platform"],
];

/** umu's id for a game it has no fix for. */
const NO_FIX = "umu-default";

/** A fix in a few words. A bare number is a Steam AppID, whose store does not matter. */
export function describeFix(fix: GameFix): string {
  if (fix.id === NO_FIX) return "no fix";
  if (/^\d+$/.test(fix.id)) return `Steam fix ${fix.id}`;
  const store = STORES.find(([key]) => key === fix.store)?.[1] ?? fix.store;
  return fix.store === "none" ? fix.id : `${fix.id} (${store})`;
}

/** Choosing a game's Proton fix by hand, for a game the umu list does not match on its own. */
export function FixPickerDialog({
  gameTitle,
  steamFix,
  onChoose,
  onClose,
}: {
  gameTitle: string;
  /** The fix under the game's Steam AppID, when the Proton build has one. */
  steamFix: GameFix | null;
  /** Saves the choice, and throws when the backend refuses it. */
  onChoose: (fix: GameFix) => Promise<void>;
  onClose: () => void;
}) {
  const [query, setQuery] = useState(gameTitle);
  const [results, setResults] = useState<FixMatch[]>([]);
  const [id, setId] = useState("");
  const [store, setStore] = useState("none");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // The list is in memory, so this only waits out a burst of typing.
  useEffect(() => {
    let cancelled = false;
    const timer = setTimeout(async () => {
      try {
        const found = await backend.searchGameFixes(query);
        if (!cancelled) setResults(found ?? []);
      } catch (e) {
        if (!cancelled) setError(messageOf(e));
      }
    }, 150);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [query]);

  async function choose(fix: GameFix) {
    setBusy(true);
    setError(null);
    try {
      await onChoose(fix);
      onClose();
    } catch (e) {
      setError(messageOf(e));
      setBusy(false);
    }
  }

  const row = "block w-full px-5 py-2 text-left text-xs hover:bg-default-100 disabled:opacity-50";

  return (
    <Modal label={`Choose a Proton fix for ${gameTitle}`} fitted onDismiss={onClose}>
      <ModalHeader
        title="Choose a Proton fix"
        description={
          <>
            Proton applies a workaround when it knows which game it is running. Pick{" "}
            <strong>{gameTitle}</strong> from the list, or type an id.
          </>
        }
      />
      <div className="px-5 pb-4">
        <TextInput
          autoFocus
          icon="search"
          placeholder="Search by name or id"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto border-t border-default-200/60">
        {steamFix && (
          <button type="button" disabled={busy} onClick={() => choose(steamFix)} className={row}>
            <span className="block">The fix for this game&rsquo;s Steam release</span>
            <span className="block font-mono text-[0.6875rem] text-foreground/50">
              {describeFix(steamFix)}
            </span>
          </button>
        )}
        {results.map(({ title, fix }) => (
          <button
            key={`${fix.store}:${fix.id}:${title}`}
            type="button"
            disabled={busy}
            onClick={() => choose(fix)}
            className={row}
          >
            <span className="block">{title}</span>
            <span className="block font-mono text-[0.6875rem] text-foreground/50">
              {describeFix(fix)}
            </span>
          </button>
        ))}
        {results.length === 0 && !steamFix && (
          <p className="px-5 py-3 text-xs text-foreground/50">
            Nothing in the list matches. Type an id below, or leave the game on automatic.
          </p>
        )}
      </div>

      <div className="flex flex-col gap-1.5 border-t border-default-200/60 px-5 py-3">
        <div className="flex gap-2">
          <TextInput
            mono
            className="flex-1"
            aria-label="Fix id"
            placeholder="Id or Steam AppID"
            spellCheck={false}
            value={id}
            onChange={(e) => setId(e.target.value)}
          />
          <Select aria-label="Store" value={store} onChange={(e) => setStore(e.target.value)}>
            {STORES.map(([key, label]) => (
              <option key={key} value={key}>
                {label}
              </option>
            ))}
          </Select>
          <Button disabled={busy || !id.trim()} onClick={() => choose({ id, store })}>
            Use
          </Button>
        </div>
        <p className={HINT}>
          A number alone is a Steam AppID, and the store is ignored. Other ids are filed under
          the store they were made for.
        </p>
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
