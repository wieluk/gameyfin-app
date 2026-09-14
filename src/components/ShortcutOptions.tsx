import { useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Alert } from "@/components/Alert";
import { Button } from "@/components/ui";
import { backend, type ShortcutLocation } from "@/lib/backend";
import { messageOf } from "@/lib/errors";

/**
 * Where an installed game can be launched from. Shortcuts run Gameyfin with `--launch <id>`,
 * not the `.exe`, so prefix, runtime and playtime tracking still apply.
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
      // Steam only reads its shortcuts file at startup, so say to restart it.
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
      <p className="mb-1 text-xs text-foreground/55">Add to</p>
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
        <Alert inline className="mt-1.5">
          {error}
        </Alert>
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
    <Button size="sm" pressed={on} onClick={onClick} disabled={busy || disabled}>
      {busy ? "…" : label}
    </Button>
  );
}
