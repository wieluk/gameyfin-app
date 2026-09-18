import { Row, Section } from "./controls";
import { Alert } from "@/components/Alert";
import { Button, SwitchField } from "@/components/ui";
import { updateChannelNote, useInstallUpdate, useUpdate } from "@/components/UpdateBanner";
import { backend } from "@/lib/backend";
import { useAppSettings, useSettingsUpdate } from "@/lib/queries";
import { HINT } from "@/lib/ui";

export function AboutSection() {
  const update = useUpdate();
  const { busy, outcome, error, install, restart } = useInstallUpdate();
  const settings = useAppSettings();
  const save = useSettingsUpdate();
  const status = update.data;

  return (
    <Section title="About" help="updates">
      <Row label="Version" value={status?.currentVersion ?? "…"} />
      <Row
        label="Latest release"
        value={
          update.isLoading
            ? "Checking…"
            : status?.error
              ? "Could not check"
              : (status?.latestVersion ?? "Unknown")
        }
        tone={status?.available ? "bad" : undefined}
      />

      <div className="flex flex-wrap gap-2 pt-1">
        <Button disabled={update.isFetching} onClick={() => void update.refetch()}>
          {update.isFetching ? "Checking…" : "Check now"}
        </Button>
        {outcome?.restartNeeded ? (
          <Button variant="primary" disabled={busy} onClick={() => void restart()}>
            {busy ? "Restarting…" : outcome.message}
          </Button>
        ) : (
          status?.available &&
          status.canInstall && (
            <Button variant="primary" disabled={busy} onClick={() => void install()}>
              {busy ? "Updating…" : `Update to ${status.latestVersion}`}
            </Button>
          )
        )}
        {status && (
          <Button onClick={() => void backend.openUrl(status.releaseUrl)}>Release notes</Button>
        )}
      </div>

      {outcome && !outcome.restartNeeded && (
        <p className="text-[11px] text-foreground/60">{outcome.message}</p>
      )}
      {error && <Alert inline>{error}</Alert>}

      <SwitchField
        label="Check for updates at startup"
        hint="One request to GitHub when the app opens. Nothing is downloaded until you ask."
        checked={settings.data?.checkForUpdates ?? true}
        onChange={(next) => save({ checkForUpdates: next })}
      />

      {status && <p className={HINT}>{updateChannelNote(status)}</p>}
    </Section>
  );
}
