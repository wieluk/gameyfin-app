import { useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Icon } from "@/components/Icon";
import { backend, isMockBackend, type UpdateStatus } from "@/lib/backend";
import { messageOf } from "@/lib/errors";

/** Read once and reused by the banner and by Settings. Invalidated as `["update"]`. */
export function useUpdate() {
  return useQuery({
    queryKey: ["update"],
    queryFn: () => backend.updateStatus(),
    // Releases are not frequent enough to be worth asking about on every window focus.
    staleTime: 60 * 60 * 1000,
    enabled: !isMockBackend,
  });
}

/**
 * A strip saying a new version exists. Formats that own their files install it; ones a package
 * manager owns link to the release page instead of a button that would do nothing.
 */
export function UpdateBanner() {
  const update = useUpdate();
  const [dismissed, setDismissed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const status = update.data;
  if (!status?.available || dismissed) return null;

  async function install() {
    setBusy(true);
    setError(null);
    try {
      setMessage(await backend.installUpdate());
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex shrink-0 flex-wrap items-center justify-center gap-x-3 gap-y-1 bg-primary/15 px-4 py-1.5 text-[11px] text-primary">
      <span className="flex items-center gap-2">
        <Icon name="download" className="h-3.5 w-3.5 shrink-0" />
        Gameyfin {status.latestVersion} is available. You have {status.currentVersion}.
      </span>

      {message ? (
        <span className="text-foreground/70">{message}</span>
      ) : status.canInstall ? (
        <button
          type="button"
          disabled={busy}
          onClick={() => void install()}
          className="rounded-lg bg-primary px-2.5 py-0.5 text-[11px] font-medium text-white transition-colors hover:bg-primary-600 disabled:opacity-50"
        >
          {busy ? "Updating…" : "Update now"}
        </button>
      ) : (
        <button
          type="button"
          onClick={() => void backend.openUrl(status.releaseUrl)}
          className="rounded-lg border border-primary/40 px-2.5 py-0.5 text-[11px] font-medium transition-colors hover:bg-primary/10"
        >
          {status.channel === "system-package" ? "How to update" : "Open the release"}
        </button>
      )}

      {error && <span className="text-danger">{error}</span>}

      <button
        type="button"
        onClick={() => setDismissed(true)}
        className="text-foreground/40 underline-offset-2 hover:text-foreground/70 hover:underline"
      >
        Not now
      </button>
    </div>
  );
}

/** A sentence explaining what this package format can and cannot do about updating. */
export function updateChannelNote(status: UpdateStatus): string {
  switch (status.channel) {
    case "self-install":
      return "Gameyfin can download and apply updates itself.";
    case "flatpak":
      return "Updates come from the Gameyfin Flatpak repository, so your system installs them alongside everything else. Updating from here runs the same thing.";
    case "system-package":
      return "This copy was installed by your system's package manager, which owns the files, so updates arrive with the rest of your system updates. Gameyfin can only tell you a new version exists.";
    default:
      return "This is a build from source, so there is nothing to update it to.";
  }
}
