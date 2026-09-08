import { useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { backend, type ShortcutLocation } from "@/lib/backend";
import { messageOf } from "@/lib/errors";

/**
 * Where an installed game can be launched from, besides this app.
 *
 * All three shortcuts run Gameyfin with `--launch <id>` rather than the game's executable,
 * so the prefix is prepared, the right runtime is picked and playtime is still recorded.
 * A shortcut straight to the `.exe` would do none of that.
 */
export function ShortcutOptions({ gameId }: { gameId: number }) {
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const status = useQuery({
    queryKey: ["shortcuts", gameId],
    queryFn: () => backend.shortcutStatus(gameId),
  });

  async function toggle(location: ShortcutLocation, enabled: boolean) {
    setBusy(location);
    setError(null);
    setNote(null);
    try {
      await backend.setShortcut(gameId, location, enabled);
      await status.refetch();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(null);
    }
  }

  async function toggleSteam(enabled: boolean) {
    setBusy("steam");
    setError(null);
    setNote(null);
    try {
      // The message matters here: Steam reads its shortcuts file at startup, so the game
      // does not appear until it is restarted, and without saying so the button looks
      // like it failed.
      setNote(await backend.setSteamShortcut(gameId, enabled));
      await status.refetch();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(null);
    }
  }

  const data = status.data;

  return (
    <div>
      <p className="mb-1.5 text-[11px] text-foreground/45">Add to</p>
      <div className="flex flex-wrap gap-1.5">
        <Toggle
          label="Applications menu"
          on={data?.menu ?? false}
          busy={busy === "menu"}
          disabled={!data}
          onClick={() => void toggle("menu", !data?.menu)}
        />
        <Toggle
          label="Desktop"
          on={data?.desktop ?? false}
          busy={busy === "desktop"}
          disabled={!data}
          onClick={() => void toggle("desktop", !data?.desktop)}
        />
        {/* Only offered where Steam exists; a button that can only ever fail is worse
            than no button. */}
        {data?.steamAvailable && (
          <Toggle
            label="Steam"
            on={data.steam}
            busy={busy === "steam"}
            disabled={false}
            onClick={() => void toggleSteam(!data.steam)}
          />
        )}
      </div>

      {note && <p className="mt-1.5 text-[11px] text-foreground/50">{note}</p>}
      {error && (
        <p role="alert" className="mt-1.5 text-[11px] leading-relaxed text-danger">
          {error}
        </p>
      )}
    </div>
  );
}

function Toggle({
  label,
  on,
  busy,
  disabled,
  onClick,
}: {
  label: string;
  on: boolean;
  busy: boolean;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={busy || disabled}
      aria-pressed={on}
      className={`rounded-lg border px-2.5 py-1 text-[11px] transition-colors disabled:opacity-50 ${
        on
          ? "border-primary/40 bg-primary/15 text-primary"
          : "border-default-200 text-foreground/70 hover:bg-default-100"
      }`}
    >
      {busy ? "…" : label}
    </button>
  );
}
