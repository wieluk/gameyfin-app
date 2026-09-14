import { PathRow, Section } from "./controls";
import { Alert } from "@/components/Alert";
import { Button, FormField, Select } from "@/components/ui";
import { backend } from "@/lib/backend";
import type { LogLevel } from "@/bindings/LogLevel";
import { messageOf } from "@/lib/errors";
import { formatBytes } from "@/lib/format";
import { useAppSettings, useSettingsUpdate } from "@/lib/queries";
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

  const current = level ?? settings.data?.logLevel ?? "info";

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
    <Section title="Diagnostics">
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
    </Section>
  );
}
