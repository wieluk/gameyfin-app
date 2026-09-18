import { useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Alert } from "@/components/Alert";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { TransferProgress } from "@/components/TransferProgress";
import { Button, FormField, Select, SwitchField } from "@/components/ui";
import { isInstalled } from "@/lib/actions";
import { backend } from "@/lib/backend";
import { formatBytes, formatRelative } from "@/lib/format";
import { keys, useAppSettings, useEntries, useInvalidate, useProtonStatus } from "@/lib/queries";
import { useAction } from "@/lib/useAction";
import { useTauriEvent } from "@/lib/useTauriEvent";
import { HINT } from "@/lib/ui";
import type { InstalledProton } from "@/bindings/InstalledProton";
import type { InstallerMemoryLimit } from "@/bindings/InstallerMemoryLimit";
import type { PrefixEntry } from "@/bindings/PrefixEntry";
import type { ProtonFamily } from "@/bindings/ProtonFamily";
import type { ProtonRelease } from "@/bindings/ProtonRelease";
import type { TransferProgress as Transfer } from "@/bindings/TransferProgress";
import { Row, SaveError, Section, useSettingSaver } from "./controls";

/** Proton, which runs Windows games, and the 32-bit support installers need. */
export function ProtonSection() {
  const invalidate = useInvalidate();
  const action = useAction();
  const [busy, setBusy] = useState<ProtonFamily | "i386" | null>(null);
  const [progress, setProgress] = useState<Transfer | null>(null);
  // Kept until the user leaves the page, since it asks for a restart.
  const [i386Result, setI386Result] = useState<string | null>(null);

  const status = useProtonStatus();
  const proton = status.data;
  const downloading = busy === "umu-proton" || busy === "ge-proton";
  useTauriEvent<Transfer>("proton-progress", setProgress, downloading);

  async function run(key: ProtonFamily | "i386", work: () => Promise<unknown>) {
    setBusy(key);
    setProgress(null);
    await action.run(async () => {
      await work();
      await status.refetch();
      // The per-game picker lists the builds too.
      await invalidate(keys.gameOptionsAll, keys.installPlans);
    });
    setBusy(null);
    setProgress(null);
  }

  const installedOf = (family: ProtonFamily) =>
    proton?.installed.find((build) => build.family === family);

  return (
    <Section title="Proton" help="windowsGames">
      {proton?.launcherProblem && (
        <Alert inline>
          {proton.launcherProblem} Windows games run on Wine until this is fixed.
        </Alert>
      )}
      <div className="flex flex-col gap-1.5">
        <BuildRow
          label="UMU-Proton"
          hint="What every game runs on, unless it picks GE-Proton."
          installed={installedOf("umu-proton")}
          latest={proton?.latestUmu}
          disabled={busy !== null}
          onDownload={() => void run("umu-proton", () => backend.installProton("umu-proton"))}
        />
        <BuildRow
          label="GE-Proton"
          hint="Optional. Adds media codecs some cutscenes need."
          installed={installedOf("ge-proton")}
          latest={proton?.latestGe}
          disabled={busy !== null}
          onDownload={() => void run("ge-proton", () => backend.installProton("ge-proton"))}
          onRemove={(name) => void run("ge-proton", () => backend.removeProton(name))}
        />
      </div>
      {downloading && (
        <TransferProgress
          receivedBytes={progress?.receivedBytes ?? 0}
          totalBytes={progress?.totalBytes ?? 0}
          bytesPerSecond={progress?.bytesPerSecond}
          label={progress ? undefined : "Starting download…"}
        />
      )}

      <Row
        label="32-bit programs"
        value={
          status.isLoading
            ? "…"
            : !proton?.supports32bit
              ? "Not supported"
              : proton.supports32bitGraphics
                ? "Supported"
                : "No 32-bit graphics driver"
        }
        tone={
          proton ? (proton.supports32bit && proton.supports32bitGraphics ? "good" : "bad") : undefined
        }
      />
      {proton?.missingI386Extension && (
        <div className="flex flex-col gap-1.5">
          <p className={HINT}>
            Flathub&rsquo;s 32-bit libraries and graphics drivers let installers and older games
            run in Proton. Without the libraries they run on Gameyfin&rsquo;s own Wine, and
            without the drivers a 32-bit program that draws with OpenGL cannot open its window.
          </p>
          <div>
            <Button
              disabled={busy !== null}
              onClick={() =>
                void run("i386", async () => setI386Result(await backend.install32bitSupport()))
              }
            >
              {busy === "i386" ? "Installing…" : "Install 32-bit support"}
            </Button>
          </div>
        </div>
      )}
      {i386Result && <p className="text-[11px] text-success-600">{i386Result}</p>}
      <SaveError error={action.error} />
      <p className={HINT}>
        Windows games run in Valve&rsquo;s Steam Runtime with Proton, the way Steam runs them.
        UMU-Proton downloads on the first launch, and a newer build replaces the old one.
      </p>
    </Section>
  );
}

/** One Proton family: what is installed, and the download or update on offer. */
function BuildRow({
  label,
  hint,
  installed,
  latest,
  disabled,
  onDownload,
  onRemove,
}: {
  label: string;
  hint: string;
  installed: InstalledProton | undefined;
  latest: ProtonRelease | null | undefined;
  disabled: boolean;
  onDownload: () => void;
  onRemove?: (name: string) => void;
}) {
  // A build's folder can carry an architecture suffix its tag lacks.
  const outdated = Boolean(installed && latest && !installed.name.startsWith(latest.tag));
  return (
    <div className="flex items-center justify-between gap-2 rounded-lg border border-default-200 bg-content2 px-2.5 py-1.5">
      <div className="min-w-0">
        <p className="truncate text-xs text-foreground">{installed?.name ?? label}</p>
        <p className="text-[11px] text-foreground/45">{hint}</p>
      </div>
      <div className="flex shrink-0 gap-1.5">
        {latest && (!installed || outdated) && (
          <Button size="sm" disabled={disabled} onClick={onDownload}>
            {installed ? `Update to ${latest.tag}` : `Download (${formatBytes(latest.sizeBytes)})`}
          </Button>
        )}
        {installed && onRemove && (
          <Button
            size="sm"
            variant="destructive"
            disabled={disabled}
            onClick={() => onRemove(installed.name)}
          >
            Remove
          </Button>
        )}
      </div>
    </div>
  );
}

/** Per-title Proton fixes. */
export function UmuSection() {
  const settings = useAppSettings();
  const { save, error: saveError } = useSettingSaver();
  const action = useAction();
  const status = useQuery({ queryKey: keys.umu, queryFn: () => backend.umuStatus() });
  const age = status.data?.ageSeconds;

  function refresh() {
    return action.run(async () => {
      await backend.refreshUmuDatabase();
      await status.refetch();
    });
  }

  return (
    <Section title="Game fixes" help="windowsGames">
      <Row
        label="Known games"
        value={
          status.isLoading
            ? "…"
            : status.data?.entries
              ? `${status.data.entries.toLocaleString()} titles`
              : "Not downloaded yet"
        }
        tone={status.data?.entries ? "good" : undefined}
      />
      <Row
        label="Updated"
        value={
          status.isLoading
            ? "…"
            : age == null
              ? "Never"
              : formatRelative(new Date(Date.now() - age * 1000).toISOString())
        }
      />
      <SwitchField
        label="Apply per-title Proton fixes"
        hint="Passes each game's id to Proton so per-title workarounds apply. Matched by Steam AppID where your server knows one, by title otherwise."
        checked={settings.data?.umuFixes ?? true}
        onChange={(next) => save({ umuFixes: next })}
      />
      <SwitchField
        label="Keep the list up to date automatically"
        hint="Fetches a fresh copy once a day, at startup and while Gameyfin keeps running."
        checked={settings.data?.umuAutoUpdate ?? true}
        onChange={(next) => save({ umuAutoUpdate: next })}
      />
      <div className="pt-1">
        <Button disabled={action.busy} onClick={() => void refresh()}>
          {action.busy ? "Downloading…" : "Update now"}
        </Button>
      </div>
      <SaveError error={action.error ?? saveError} />
      <p className={HINT}>A game not in the list runs as it would with fixes off.</p>
    </Section>
  );
}

/** Enough steps to find a working limit by bisection. */
const MEMORY_LIMITS = [1024, 1536, 2048, 3072, 4096, 6144, 8192, 12288, 16384, 24576, 32768];

function gigabytes(mib: number): string {
  return `${Number((mib / 1024).toFixed(1))} GB`;
}

export function CompatibilitySection() {
  const settings = useAppSettings();
  const { save, error } = useSettingSaver();
  const memory = useQuery({
    queryKey: keys.memory,
    queryFn: () => backend.memoryInfo(),
    staleTime: Infinity,
  });
  const [limit, setLimit] = useState<InstallerMemoryLimit | null>(null);
  const current = limit ?? settings.data?.installerMemoryLimit ?? "auto";
  const automatic = memory.data?.automaticMib;

  async function change(value: string) {
    const next: InstallerMemoryLimit = value === "auto" || value === "off" ? value : Number(value);
    setLimit(next);
    // Dropped once saved, so a refused value does not linger as if it had been kept.
    await save({ installerMemoryLimit: next });
    setLimit(null);
  }

  return (
    <Section title="Compatibility" help="windowsGames">
      <FormField
        label="Installer memory limit"
        htmlFor="installer-memory"
        hint="Repack installers take all the RAM they find, which can hang the whole machine. Automatic caps them at half your RAM, never below 4 GB. If one still gets stuck, try 3 GB."
      >
        <Select
          id="installer-memory"
          value={String(current)}
          onChange={(e) => void change(e.target.value)}
        >
          <option value="auto">
            Automatic{automatic ? `, ${gigabytes(automatic)} here` : ""} (default)
          </option>
          {MEMORY_LIMITS.map((mb) => (
            <option key={mb} value={mb}>
              {mb / 1024} GB
            </option>
          ))}
          <option value="off">No limit</option>
        </Select>
      </FormField>
      <SaveError error={error} />
    </Section>
  );
}

/** Prefixes of games no longer installed. An installed game's prefix is in its own options. */
export function PrefixSection() {
  const action = useAction();
  const [confirming, setConfirming] = useState<PrefixEntry | null>(null);
  const prefixes = useQuery({ queryKey: keys.prefixes, queryFn: () => backend.listPrefixes() });
  const entries = useEntries();

  const installed = new Set((entries.data ?? []).filter(isInstalled).map((e) => e.game.id));
  const rows = (prefixes.data ?? []).filter((prefix) => !installed.has(prefix.gameId));

  return (
    <Section title="Leftover prefixes" help="prefixes">
      {rows.length === 0 ? (
        <p className="text-[11px] text-foreground/45">
          {prefixes.isLoading ? "…" : "None. Every prefix belongs to an installed game."}
        </p>
      ) : (
        <div className="flex flex-col gap-2">
          {rows.map((prefix) => (
            <div
              key={prefix.gameId}
              className="flex items-center justify-between gap-3 rounded-lg border border-default-200 bg-content2 px-2.5 py-1.5"
            >
              <span className="min-w-0 truncate text-xs text-foreground" title={prefix.path}>
                {prefix.title ?? `Game ${prefix.gameId}`}, {formatBytes(prefix.bytes)}
              </span>
              <Button size="sm" variant="destructive" onClick={() => setConfirming(prefix)}>
                Delete
              </Button>
            </div>
          ))}
        </div>
      )}

      <SaveError error={action.error} />

      <p className={HINT}>
        Left behind by games that are no longer installed. An installed game&rsquo;s prefix is
        in its options under Installed.
      </p>

      {confirming && (
        <ConfirmDialog
          title={`Delete the prefix for ${confirming.title ?? `game ${confirming.gameId}`}?`}
          body={
            <>
              Anything the game saved <em>inside</em> the prefix is removed with it, which for
              some Windows games includes save files.
            </>
          }
          confirmLabel="Delete the prefix"
          onConfirm={() => {
            const target = confirming;
            setConfirming(null);
            void action.run(async () => {
              await backend.deletePrefix(target.gameId);
              await prefixes.refetch();
            });
          }}
          onCancel={() => setConfirming(null)}
        />
      )}
    </Section>
  );
}
