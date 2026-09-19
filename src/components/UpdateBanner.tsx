import { useState } from "react";
import { useQuery } from "@tanstack/react-query";

import type { UpdateOutcome } from "@/bindings/UpdateOutcome";
import type { UpdateProgress } from "@/bindings/UpdateProgress";
import { Icon } from "@/components/Icon";
import { Button } from "@/components/ui";
import { backend, isMockBackend, type UpdateStatus } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { formatBytes } from "@/lib/format";
import { useTauriEvent } from "@/lib/useTauriEvent";

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

/** Installing and the restart that follows it, shared by the banner and Settings. */
export function useInstallUpdate() {
  const [busy, setBusy] = useState(false);
  const [outcome, setOutcome] = useState<UpdateOutcome | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [progress, setProgress] = useState<UpdateProgress | null>(null);

  // Subscribed only while something is running, so a stale step cannot reappear later.
  useTauriEvent<UpdateProgress>("update-progress", setProgress, busy);

  async function run(action: () => Promise<void>) {
    setBusy(true);
    setError(null);
    setProgress(null);
    try {
      await action();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(false);
      setProgress(null);
    }
  }

  return {
    busy,
    outcome,
    error,
    progress,
    install: () => run(async () => setOutcome(await backend.installUpdate())),
    restart: () => run(() => backend.restartApp()),
  };
}

/** How far along an install is: a bar, plus whatever the format could count. */
export function UpdateProgressBar({ progress }: { progress: UpdateProgress | null }) {
  const total = progress?.totalBytes ?? 0;
  const done =
    progress?.percent ?? (total > 0 ? ((progress?.receivedBytes ?? 0) / total) * 100 : null);

  return (
    <span className="flex items-center gap-2">
      <span className="h-1.5 w-24 overflow-hidden rounded-full bg-default-200">
        <span
          className={`block h-full rounded-full bg-primary transition-[width] ${done === null ? "animate-pulse" : ""}`}
          style={{ width: done === null ? "100%" : `${Math.round(done)}%` }}
        />
      </span>
      <span className="text-foreground/70">
        {progress?.phase ?? "Starting…"}
        {done !== null && ` ${Math.round(done)}%`}
        {total > 0 && ` (${formatBytes(progress?.receivedBytes ?? 0)} of ${formatBytes(total)})`}
      </span>
    </span>
  );
}

/**
 * A strip saying a new version exists. Formats that own their files install it; ones a package
 * manager owns link to the release page instead of a button that would do nothing.
 */
export function UpdateBanner() {
  const update = useUpdate();
  const [dismissed, setDismissed] = useState(false);
  const { busy, outcome, error, progress, install, restart } = useInstallUpdate();

  const status = update.data;
  if (!status?.available || dismissed) return null;

  return (
    <div className="flex shrink-0 flex-wrap items-center justify-center gap-x-3 gap-y-1 bg-primary/15 px-4 py-1.5 text-[11px] text-primary">
      <span className="flex items-center gap-2">
        <Icon name="download" className="h-3.5 w-3.5 shrink-0" />
        Gameyfin {status.latestVersion} is available. You have {status.currentVersion}.
      </span>

      {outcome?.restartNeeded ? (
        <Button size="sm" variant="primary" disabled={busy} onClick={() => void restart()}>
          {busy ? "Restarting…" : outcome.message}
        </Button>
      ) : outcome ? (
        <span className="text-foreground/70">{outcome.message}</span>
      ) : busy ? (
        <UpdateProgressBar progress={progress} />
      ) : status.canInstall ? (
        <Button size="sm" variant="primary" onClick={() => void install()}>
          Update now
        </Button>
      ) : (
        <Button size="sm" onClick={() => void backend.openUrl(status.releaseUrl)}>
          {status.channel === "system-package" ? "How to update" : "Open the release"}
        </Button>
      )}

      {error && <span className="text-danger">{error}</span>}

      <Button size="sm" variant="ghost" onClick={() => setDismissed(true)}>
        Not now
      </Button>
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
