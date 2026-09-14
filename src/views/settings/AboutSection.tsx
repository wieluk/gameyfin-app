import { Check, Row, Section } from "./controls";
import { updateChannelNote, useUpdate } from "@/components/UpdateBanner";
import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { useAppSettings, useSettingsUpdate } from "@/lib/queries";
import { BUTTON, BUTTON_MAYBE_DISABLED, HINT } from "@/lib/ui";
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
        <button
          type="button"
          disabled={update.isFetching}
          onClick={() => void update.refetch()}
          className={BUTTON_MAYBE_DISABLED}
        >
          {update.isFetching ? "Checking…" : "Check now"}
        </button>
        {status?.available && status.canInstall && (
          <button
            type="button"
            disabled={busy}
            onClick={() => void install()}
            className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-primary-600 disabled:opacity-50"
          >
            {busy ? "Updating…" : `Update to ${status.latestVersion}`}
          </button>
        )}
        {status && (
          <button
            type="button"
            onClick={() => void backend.openUrl(status.releaseUrl)}
            className={BUTTON}
          >
            Release notes
          </button>
        )}
      </div>

      {message && <p className="text-[11px] text-foreground/60">{message}</p>}
      {error && (
        <p role="alert" className="text-[11px] text-danger">
          {error}
        </p>
      )}

      <Check
        label="Check for updates at startup"
        hint="One request to GitHub when the app opens. Nothing is downloaded until you ask."
        checked={settings.data?.checkForUpdates ?? true}
        onChange={(next) => save({ checkForUpdates: next })}
      />

      {status && (
        <p className={HINT}>
          {updateChannelNote(status)}
        </p>
      )}
    </Section>
  );
}
