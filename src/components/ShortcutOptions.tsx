import { useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Alert } from "@/components/Alert";
import { Button } from "@/components/ui";
import { backend, type ShortcutLocation } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { isWindows } from "@/lib/platform";

/**
 * Where an installed game can be launched from. Shortcuts run Gameyfin with `--launch <id>`,
 * not the `.exe`, so prefix, runtime and playtime tracking still apply.
 */
export function ShortcutOptions({
  gameId,
  inline = false,
}: {
  gameId: number;
  /** Label beside the toggles rather than above, to sit in a row of buttons. */
  inline?: boolean;
}) {
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
  const size = inline ? "md" : "sm";

  return (
    <div>
      <div
        role="group"
        aria-label="Add to"
        className={inline ? "flex flex-wrap items-center gap-1.5" : "flex flex-col gap-1"}
      >
        <span className={`text-xs ${inline ? "mr-0.5 text-foreground/45" : "text-foreground/55"}`}>
          Add to
        </span>
        <div className="flex flex-wrap gap-1.5">
          <Toggle
            size={size}
            label={isWindows ? "Start menu" : "Applications menu"}
            on={data?.menu ?? false}
            busy={busy === "menu"}
            disabled={!data}
            onClick={() => void toggle("menu", !data?.menu)}
          />
          <Toggle
            size={size}
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
              size={size}
              label="Steam"
              on={data.steam}
              busy={busy === "steam"}
              disabled={false}
              onClick={() => void toggleSteam(!data.steam)}
            />
          )}
        </div>
      </div>

      {note && <p className="mt-1.5 text-[0.6875rem] text-foreground/50">{note}</p>}
      {error && (
        <Alert inline className="mt-1.5">
          {error}
        </Alert>
      )}
    </div>
  );
}

function Toggle({
  size,
  label,
  on,
  busy,
  disabled,
  onClick,
}: {
  size: "sm" | "md";
  label: string;
  on: boolean;
  busy: boolean;
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <Button
      size={size}
      pressed={on}
      icon={on && !busy ? "check" : undefined}
      onClick={onClick}
      disabled={busy || disabled}
    >
      {busy ? "…" : label}
    </Button>
  );
}
