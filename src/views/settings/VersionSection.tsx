import { Row, Section } from "./controls";
import { Alert } from "@/components/Alert";
import { Button, FormField, Select } from "@/components/ui";
import { useTauriEvent } from "@/lib/useTauriEvent";
import type { TransferProgress } from "@/bindings/TransferProgress";
import { messageOf } from "@/lib/errors";
import { formatBytes, formatSpeed } from "@/lib/format";
import { useState } from "react";

/** What a downloadable helper looks like to the section below. */
export interface VersionInfo {
  /** The version in use, or null when nothing is installed. */
  version: string | null;
  /** How to show it, when the version alone is not enough, as with Wine's build. */
  label?: string;
  /** True when what is in use ships with the app, so it cannot be removed. */
  builtIn: boolean;
  /** Null when the release feed could not be reached, which is not "up to date". */
  latest: string | null;
  /** Download size of the latest release, when known. */
  downloadBytes: number | null;
  /** Recent versions, newest first. */
  available: string[];
  /** Versions in labelled lists, shown in place of `available`. */
  groups?: Array<{ label: string; versions: string[] }>;
  /** How to name the latest release, when the version alone is not enough. */
  latestLabel?: string;
  /** A second install to offer beside the main one, such as a way back to stable. */
  alternative?: { version: string; label: string } | null;
  updatable: boolean;
}

export interface VersionTool {
  /** Section heading, and the noun the buttons use. */
  title: string;
  /** Namespaces this section's form controls.  */
  id: string;
  /** Tauri event carrying download progress. */
  progressEvent: string;
  /** What to show. Owned by the caller, since sections sharing a query key would mix shapes. */
  info?: VersionInfo;
  loading: boolean;
  install(version?: string): Promise<void>;
  remove(): Promise<void>;
  /** Re-read the status, plus whatever else depended on the tool being present. */
  after(): Promise<void>;
  /** Advice under the version picker, in place of the generic line. */
  versionHint?: string;
}

/** Install, update, remove and pin a helper the app downloads for itself. */
export function VersionSection({ tool, children }: { tool: VersionTool; children?: React.ReactNode }) {
  const [busy, setBusy] = useState<"install" | "remove" | null>(null);
  const [progress, setProgress] = useState<TransferProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [chosen, setChosen] = useState("");

  const info = tool.info;

  // Subscribed only while a download is running.
  useTauriEvent<TransferProgress>(tool.progressEvent, setProgress, busy === "install");

  async function run(action: "install" | "remove", version?: string) {
    setBusy(action);
    setError(null);
    setProgress(null);
    try {
      if (action === "install") await tool.install(version ?? (chosen || undefined));
      else await tool.remove();
      await tool.after();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(null);
      setProgress(null);
    }
  }

  // A pinned version that is not the one in use is the action to offer, ahead of any update.
  const pinned = chosen && chosen !== info?.version ? chosen : null;
  const latestName = info?.latestLabel ?? info?.latest;
  const groups = info?.groups?.filter((group) => group.versions.length > 0);
  const hasChoices = groups ? groups.length > 0 : (info?.available.length ?? 0) > 0;

  return (
    <Section title={tool.title}>
      <Row
        label="Installed"
        value={
          info?.version
            ? info.builtIn
              ? `${info.label ?? info.version} (bundled)`
              : (info.label ?? info.version)
            : "Not installed"
        }
        tone={info?.version ? "good" : undefined}
      />
      <Row
        label="Latest available"
        value={
          tool.loading
            ? "Checking…"
            : (latestName ?? "Could not check, no connection")
        }
        tone={info?.updatable ? "bad" : undefined}
      />

      {busy === "install" && (
        <div className="flex flex-col gap-1 pt-1">
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-default-200">
            <div
              className="h-full bg-primary transition-[width]"
              style={{
                width: progress?.totalBytes
                  ? `${Math.round((progress.receivedBytes / progress.totalBytes) * 100)}%`
                  : "0%",
              }}
            />
          </div>
          <p className="text-[11px] text-foreground/45">
            {progress
              ? `${formatBytes(progress.receivedBytes)} of ${formatBytes(progress.totalBytes)} at ${formatSpeed(progress.bytesPerSecond)}`
              : "Starting download…"}
          </p>
        </div>
      )}

      <div className="flex flex-wrap gap-2 pt-1">
        <Button variant="primary" disabled={busy !== null} onClick={() => void run("install")}>
          {busy === "install"
            ? "Downloading…"
            : pinned
              ? `Install ${pinned}`
              : !info?.version || info.builtIn
                ? `Download ${tool.title}${info?.downloadBytes ? ` (${formatBytes(info.downloadBytes)})` : ""}`
                : info.updatable
                  ? `Update to ${info.latest}`
                  : "Redownload"}
        </Button>
        {info?.alternative && !pinned && (
          <Button
            disabled={busy !== null}
            onClick={() => void run("install", info.alternative?.version)}
          >
            {info.alternative.label}
          </Button>
        )}
        {info?.version && !info.builtIn && (
          <Button variant="destructive" disabled={busy !== null} onClick={() => void run("remove")}>
            {busy === "remove" ? "Removing…" : "Remove"}
          </Button>
        )}
      </div>

      {error && <Alert inline>{error}</Alert>}

      {hasChoices && (
        <FormField
          className="pt-2"
          label="Version"
          htmlFor={`${tool.id}-version`}
          hint={
            tool.versionHint ??
            "Pick an older version only to work around a problem with the newest one. The choice applies to the next download, not to what is installed now."
          }
        >
          <Select
            id={`${tool.id}-version`}
            value={chosen}
            onChange={(e) => setChosen(e.target.value)}
          >
            <option value="">Latest{latestName ? ` (${latestName})` : ""}</option>
            {groups
              ? groups.map((group) => (
                  <optgroup key={group.label} label={group.label}>
                    {group.versions.map((version) => (
                      <option key={version} value={version}>
                        {version}
                      </option>
                    ))}
                  </optgroup>
                ))
              : info?.available.map((version) => (
                  <option key={version} value={version}>
                    {version}
                  </option>
                ))}
          </Select>
        </FormField>
      )}

      {children}
    </Section>
  );
}
