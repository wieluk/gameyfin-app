import { PathRow, Section } from "./controls";
import { backend } from "@/lib/backend";
import type { LogLevel } from "@/bindings/LogLevel";
import { messageOf } from "@/lib/errors";
import { formatBytes } from "@/lib/format";
import { useAppSettings, useSettingsUpdate } from "@/lib/queries";
import { INPUT } from "@/lib/ui";
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
      <label className="text-xs text-foreground/55" htmlFor="log-level">
        Log detail
      </label>
      <select
        id="log-level"
        value={current}
        onChange={(e) => void change(e.target.value as LogLevel)}
        className={INPUT}
      >
        {LOG_LEVELS.map((option) => (
          <option key={option.key} value={option.key}>
            {option.label}
          </option>
        ))}
      </select>
      <p className="text-[11px] text-foreground/45">
        Applies immediately. Use Debug while reproducing a problem, then attach the log.
      </p>
      {levelError && (
        <p role="alert" className="text-xs text-danger">
          {levelError}
        </p>
      )}

      <div className="mt-2 flex items-center justify-between gap-4">
        <div className="min-w-0">
          <p className="text-xs text-foreground/55">Artwork cache</p>
          <p className="text-[11px] text-foreground/45">
            {cacheSize.data === undefined
              ? "…"
              : `${formatBytes(cacheSize.data)}, cleans itself as it grows`}
          </p>
        </div>
        <button
          type="button"
          onClick={async () => {
            await backend.clearImageCache();
            await cacheSize.refetch();
          }}
          className="shrink-0 rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/70 transition-colors hover:bg-default-100"
        >
          Clear
        </button>
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
