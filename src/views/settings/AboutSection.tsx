import { Row, Section } from "./controls";
import { Alert } from "@/components/Alert";
import { Button, SwitchField } from "@/components/ui";
import { updateChannelNote, useUpdate } from "@/components/UpdateBanner";
import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { useAppSettings, useSettingsUpdate } from "@/lib/queries";
import { HINT } from "@/lib/ui";
import { useState } from "react";

export function AboutSection() {
  const update = useUpdate();
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const settings = useAppSettings();
  const save = useSettingsUpdate();
  const status = update.data;

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
    <Section title="About">
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
        {status?.available && status.canInstall && (
          <Button variant="primary" disabled={busy} onClick={() => void install()}>
            {busy ? "Updating…" : `Update to ${status.latestVersion}`}
          </Button>
        )}
        {status && (
          <Button onClick={() => void backend.openUrl(status.releaseUrl)}>Release notes</Button>
        )}
      </div>

      {message && <p className="text-[11px] text-foreground/60">{message}</p>}
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
