import { PathRow, Section } from "./controls";
import { Alert } from "@/components/Alert";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Button, FormField, Select } from "@/components/ui";
import { backend } from "@/lib/backend";
import type { LogLevel } from "@/bindings/LogLevel";
import { messageOf } from "@/lib/errors";
import { formatBytes } from "@/lib/format";
import { keys, useAppSettings, useInvalidate, useSettingsUpdate } from "@/lib/queries";
import { useAction } from "@/lib/useAction";
import { useQuery } from "@tanstack/react-query";
import { useState } from "react";

export const LOG_LEVELS = [
  { key: "error", label: "Error" },
  { key: "warn", label: "Warning" },
  { key: "info", label: "Info" },
  { key: "debug", label: "Debug" },
  { key: "trace", label: "Trace" },
];

export function DiagnosticsSection() {
  const logs = useQuery({ queryKey: ["log-dir"], queryFn: () => backend.logDirectory() });
  const settings = useAppSettings();
  const save = useSettingsUpdate();
  const cacheSize = useQuery({ queryKey: ["image-cache"], queryFn: () => backend.imageCacheSize() });
  const configDir = useQuery({ queryKey: ["config-dir"], queryFn: () => backend.configDirectory() });
  const [level, setLevel] = useState<LogLevel | null>(null);
  const [levelError, setLevelError] = useState<string | null>(null);

  const reset = useAction();
  const invalidate = useInvalidate();
  const [confirmReset, setConfirmReset] = useState(false);

  const current = level ?? settings.data?.logLevel ?? "info";

  async function resetSettings() {
    setConfirmReset(false);
    await reset.run(async () => {
      await backend.resetSettings();
      await settings.refetch();
      // Save sync may have been off, which changes every save row.
      await invalidate(keys.saveOverviewAll);
    });
  }

  async function change(next: LogLevel) {
    setLevel(next);
    setLevelError(null);
    try {
      await save({ logLevel: next });
    } catch (e) {
      setLevelError(messageOf(e));
    } finally {
      // The stored value is the truth once the save has been attempted.
      setLevel(null);
    }
  }

  return (
    <Section title="Diagnostics" help="troubleshooting">
      <FormField
        label="Log detail"
        htmlFor="log-level"
        hint="Applies immediately. Use Debug while reproducing a problem, then attach the log."
      >
        <Select
          id="log-level"
          value={current}
          onChange={(e) => void change(e.target.value as LogLevel)}
        >
          {LOG_LEVELS.map((option) => (
            <option key={option.key} value={option.key}>
              {option.label}
            </option>
          ))}
        </Select>
      </FormField>
      {levelError && <Alert inline>{levelError}</Alert>}

      <div className="mt-2 flex items-center justify-between gap-4">
        <div className="min-w-0">
          <p className="text-xs text-foreground/55">Artwork cache</p>
          <p className="text-[11px] text-foreground/45">
            {cacheSize.data === undefined
              ? "…"
              : `${formatBytes(cacheSize.data)}, cleans itself as it grows`}
          </p>
        </div>
        <Button
          onClick={async () => {
            await backend.clearImageCache();
            await cacheSize.refetch();
          }}
        >
          Clear
        </Button>
      </div>

      <PathRow
        label="App data"
        hint="Settings, session and local records."
        path={configDir.data}
      />

      <PathRow
        label="Log files"
        hint="One per day; attach the newest when reporting a problem."
        path={logs.data}
      />

      <div className="mt-2 flex items-center justify-between gap-4 border-t border-default-200/60 pt-3">
        <div className="min-w-0">
          <p className="text-xs text-foreground/55">Reset settings</p>
          <p className="text-[11px] text-foreground/45">
            Server, games folders, save location and per-game options stay.
          </p>
        </div>
        <Button
          variant="destructive"
          icon="reset"
          disabled={reset.busy}
          onClick={() => setConfirmReset(true)}
        >
          {reset.busy ? "Resetting…" : "Reset to defaults"}
        </Button>
      </div>
      {reset.error && <Alert inline>{reset.error}</Alert>}

      {confirmReset && (
        <ConfirmDialog
          title="Reset all settings?"
          body="Every setting goes back to its default. Your server, sign-in, games folders, save location and per-game options stay."
          confirmLabel="Reset"
          onConfirm={() => void resetSettings()}
          onCancel={() => setConfirmReset(false)}
        />
      )}
    </Section>
  );
}
