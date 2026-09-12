import { useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { ConfirmDialog } from "@/components/ConfirmDialog";
import { TransferProgress } from "@/components/TransferProgress";
import { backend } from "@/lib/backend";
import { formatBytes, formatRelative } from "@/lib/format";
import {
  keys,
  useAppSettings,
  useGraphicsStatus,
  useInvalidate,
  useProtonStatus,
  useWineStatus,
} from "@/lib/queries";
import { useAction } from "@/lib/useAction";
import { useTauriEvent } from "@/lib/useTauriEvent";
import { BUTTON_MAYBE_DISABLED, HINT, INPUT } from "@/lib/ui";
import type { InstallerMemoryLimit } from "@/bindings/InstallerMemoryLimit";
import type { PrefixEntry } from "@/bindings/PrefixEntry";
import type { ProtonRelease } from "@/bindings/ProtonRelease";
import type { TransferProgress as Transfer } from "@/bindings/TransferProgress";
import type { WineVariant } from "@/bindings/WineVariant";
import { VersionSection, type VersionTool } from "./VersionSection";
import { Check, Row, SaveError, Section, SmallButton, useSettingSaver } from "./controls";

/** The two components, as the commands name them. */
type GraphicsComponent = "dxvk" | "vkd3d";

/** Proton builds for umu, which is how Windows games run by default on Linux. */
export function ProtonSection() {
  const { save, error: saveError } = useSettingSaver();
  const invalidate = useInvalidate();
  const action = useAction();
  const [busy, setBusy] = useState<string | null>(null);
  const [progress, setProgress] = useState<Transfer | null>(null);

  const status = useProtonStatus();
  const proton = status.data;
  const downloading = busy === "umu-proton" || busy === "ge-proton";

  useTauriEvent<Transfer>("proton-progress", setProgress, downloading);

  async function run(key: string, work: () => Promise<unknown>) {
    setBusy(key);
    setProgress(null);
    await action.run(async () => {
      await work();
      await status.refetch();
      // The per-game picker reads the builds too.
      await invalidate(keys.gameOptionsAll, keys.installPlans);
    });
    setBusy(null);
    setProgress(null);
  }

  const managed = proton?.installed.filter((b) => b.source === "managed") ?? [];
  const steam = proton?.installed.filter((b) => b.source === "steam") ?? [];
  const effectiveDefault = proton?.defaultBuild ?? proton?.inUse ?? null;

  return (
    <Section title="Proton">
      <Row
        label="umu launcher"
        value={
          status.isLoading
            ? "…"
            : proton?.launcherProblem
              ? "Cannot run here"
              : `Ready${proton?.launcherVersion ? `, ${proton.launcherVersion}` : ""}`
        }
        tone={proton ? (proton.launcherProblem ? "bad" : "good") : undefined}
      />
      <Row
        label="Games use"
        value={status.isLoading ? "…" : (proton?.inUse ?? "Downloaded on first launch")}
        tone={proton?.inUse ? "good" : undefined}
      />
      {proton?.launcherProblem && (
        <p role="alert" className="text-[11px] leading-relaxed text-danger">
          {proton.launcherProblem} Windows games fall back to Wine until this is fixed.
        </p>
      )}

      {managed.length > 0 && (
        <div className="flex flex-col gap-1.5 pt-1">
          {managed.map((build) => {
            const isDefault = effectiveDefault === build.name;
            return (
              <div
                key={build.name}
                className="flex items-center justify-between gap-2 rounded-lg border border-default-200 bg-content2 px-2.5 py-1.5"
              >
                <span className="truncate text-xs text-foreground" title={build.path}>
                  {build.name}
                  {isDefault ? " (default)" : ""}
                </span>
                <div className="flex shrink-0 gap-1.5">
                  {!isDefault && (
                    <SmallButton
                      onClick={() =>
                        void run(`default:${build.name}`, () =>
                          save({ defaultProton: build.name }),
                        )
                      }
                    >
                      Make default
                    </SmallButton>
                  )}
                  <SmallButton
                    danger
                    onClick={() =>
                      void run(`remove:${build.name}`, () => backend.removeProton(build.name))
                    }
                  >
                    Remove
                  </SmallButton>
                </div>
              </div>
            );
          })}
        </div>
      )}

      <div className="flex flex-col gap-1.5 pt-1">
        <FamilyDownload
          label="UMU-Proton"
          tags={proton?.umuTags ?? []}
          latest={proton?.latestUmu}
          disabled={busy !== null}
          onDownload={(tag) => void run("umu-proton", () => backend.installProton("umu-proton", tag))}
        />
        <FamilyDownload
          label="GE-Proton"
          tags={proton?.geTags ?? []}
          latest={proton?.latestGe}
          disabled={busy !== null}
          onDownload={(tag) => void run("ge-proton", () => backend.installProton("ge-proton", tag))}
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

      <SaveError error={action.error ?? saveError} />

      <div className="pt-1">
        <SmallButton onClick={() => void save({ setupDismissed: false })}>
          Run first-time setup again
        </SmallButton>
      </div>

      {steam.length > 0 && (
        <p className={HINT}>
          Also usable per game, from Steam: {steam.map((b) => b.name).join(", ")}.
        </p>
      )}
      <p className={HINT}>
        Windows games run through umu, inside Valve&rsquo;s Steam Runtime with Proton, the way
        Steam runs them. UMU-Proton is the default; GE-Proton adds media codecs some cutscenes
        need. A game can pick its own build in its options.
      </p>
    </Section>
  );
}

/** Download one Proton family: its newest release, or a chosen older one. */
function FamilyDownload({
  label,
  tags,
  latest,
  disabled,
  onDownload,
}: {
  label: string;
  tags: string[];
  latest: ProtonRelease | null | undefined;
  disabled: boolean;
  onDownload: (tag: string | undefined) => void;
}) {
  const [tag, setTag] = useState("");
  return (
    <div className="flex items-center gap-1.5">
      <select
        aria-label={`${label} version`}
        value={tag}
        onChange={(e) => setTag(e.target.value)}
        disabled={disabled || tags.length === 0}
        className={INPUT}
      >
        <option value="">{latest ? `Newest ${label}, ${latest.tag}` : `Newest ${label}`}</option>
        {tags.map((t) => (
          <option key={t} value={t}>
            {t}
          </option>
        ))}
      </select>
      <button
        type="button"
        disabled={disabled}
        onClick={() => onDownload(tag || undefined)}
        className={BUTTON_MAYBE_DISABLED}
      >
        {latest && !tag ? `Download (${formatBytes(latest.sizeBytes)})` : "Download"}
      </button>
    </div>
  );
}

/** The Wine runtime the app downloads and keeps up to date. */
export function WineSection() {
  const settings = useAppSettings();
  const { save, error } = useSettingSaver();
  const invalidate = useInvalidate();
  const variant: WineVariant = settings.data?.wineVariant ?? "staging-wow64";

  const status = useWineStatus();
  const wine = status.data;
  const channel = wine?.installedChannel ?? "stable";
  const newestStable = wine?.stable[0];

  const tool: VersionTool = {
    title: "Wine",
    id: "wine",
    progressEvent: "wine-progress",
    loading: status.isLoading,
    info: wine && {
      version: wine.installed?.version ?? null,
      label: wine.installed
        ? `${wine.installed.version} ${channel} (${labelFor(wine.installed.variant)})`
        : undefined,
      builtIn: false,
      latest: wine.latest?.version ?? null,
      latestLabel: wine.latest ? `${wine.latest.version} ${channel}` : undefined,
      downloadBytes: wine.latest?.sizeBytes ?? null,
      available: [],
      groups: [
        { label: "Stable", versions: wine.stable },
        { label: "Development", versions: wine.development },
      ],
      // Updates stay on the installed channel, so a development build needs a way back.
      alternative:
        wine.installed && channel === "development" && newestStable
          ? { version: newestStable, label: `Switch to stable ${newestStable}` }
          : null,
      // A variant change counts: switching build is an install to perform, not a
      // version comparison.
      updatable: Boolean(
        wine.installed &&
          wine.latest &&
          (wine.installed.version !== wine.latest.version ||
            wine.installed.variant !== wine.latest.variant),
      ),
    },
    install: async (version) => void (await backend.installWine(version)),
    remove: () => backend.removeWine(),
    after: async () => {
      await status.refetch();
      // The install dialog greys out without a runtime; tell it one exists now.
      await invalidate(keys.installPlans);
    },
    versionHint:
      "Stable is recommended. Development builds are newer but can break games that worked. Updates follow the channel of the installed build.",
  };

  async function changeVariant(next: WineVariant) {
    await save({ wineVariant: next });
    await status.refetch();
  }

  return (
    <VersionSection tool={tool}>
      <label className="pt-2 text-xs text-foreground/55" htmlFor="wine-variant">
        Build
      </label>
      <select
        id="wine-variant"
        value={variant}
        onChange={(e) => void changeVariant(e.target.value as WineVariant)}
        className={INPUT}
      >
        <option value="staging-wow64">Wine-Staging, WoW64 (recommended)</option>
        <option value="staging">Wine-Staging, 32-bit libraries</option>
      </select>
      <p className={HINT}>
        The fallback: used by a game set to run with Wine in its options, or when Proton cannot
        run on this system. It has no fsync, so demanding games run noticeably slower than on
        Proton. The WoW64 build needs no 32-bit system libraries and works the same in a
        Flatpak; the other needs your distribution&rsquo;s 32-bit libraries.
      </p>
      <SaveError error={error} />
      {wine?.installed && (
        <p className={HINT}>Removing Wine leaves your game prefixes and saves untouched.</p>
      )}
    </VersionSection>
  );
}

export function labelFor(variant: WineVariant): string {
  return variant === "staging" ? "32-bit libraries" : "WoW64";
}

/** DXVK and vkd3d-proton, which are what make Direct3D work at all. */
export function GraphicsSection() {
  const settings = useAppSettings();
  const { save, error: saveError } = useSettingSaver();
  const action = useAction();
  const [busy, setBusy] = useState<GraphicsComponent | "remove" | null>(null);

  const status = useGraphicsStatus();
  const graphics = status.data;
  const enabled = settings.data?.graphicsComponents ?? true;
  const hasVulkan = graphics ? graphics.vulkan.apiVersion !== null : true;

  async function run(key: GraphicsComponent | "remove", work: () => Promise<unknown>) {
    setBusy(key);
    await action.run(async () => {
      await work();
      await status.refetch();
    });
    setBusy(null);
  }

  function versionOf(component: GraphicsComponent): string {
    const installed = graphics?.installed[component === "dxvk" ? "dxvk" : "vkd3d"];
    if (!installed) return "Not installed";
    const recommended =
      component === "dxvk" ? graphics?.recommendedDxvk : graphics?.recommendedVkd3d;
    return recommended && recommended !== installed.version
      ? `${installed.version} (${recommended} recommended)`
      : installed.version;
  }

  return (
    <Section title="Direct3D">
      <Row
        label="Vulkan driver"
        value={status.isLoading ? "…" : (graphics?.vulkanLabel ?? "Unknown")}
        tone={graphics ? (hasVulkan ? "good" : "bad") : undefined}
      />
      <Row
        label="DXVK"
        value={status.isLoading ? "…" : versionOf("dxvk")}
        tone={graphics?.installed.dxvk ? "good" : undefined}
      />
      <Row
        label="vkd3d-proton"
        value={
          status.isLoading
            ? "…"
            : graphics?.recommendedVkd3d === null && !graphics?.installed.vkd3d
              ? "Needs Vulkan 1.3"
              : versionOf("vkd3d")
        }
        tone={graphics?.installed.vkd3d ? "good" : undefined}
      />

      <div className="flex flex-wrap gap-1.5 pt-1">
        <SmallButton
          disabled={busy !== null}
          onClick={() => void run("dxvk", () => backend.installGraphics("dxvk"))}
        >
          {busy === "dxvk" ? "Downloading…" : "Update DXVK"}
        </SmallButton>
        <SmallButton
          disabled={busy !== null}
          onClick={() => void run("vkd3d", () => backend.installGraphics("vkd3d"))}
        >
          {busy === "vkd3d" ? "Downloading…" : "Update vkd3d-proton"}
        </SmallButton>
        {(graphics?.installed.dxvk || graphics?.installed.vkd3d) && (
          <SmallButton
            danger
            disabled={busy !== null}
            onClick={() => void run("remove", () => backend.removeGraphics())}
          >
            Remove
          </SmallButton>
        )}
      </div>

      <Check
        label="Install Direct3D components into each game"
        hint="Translates Direct3D to Vulkan. Without it games fall back to OpenGL and DirectX 12 titles do not start."
        checked={enabled}
        onChange={async (next) => {
          await save({ graphicsComponents: next });
          await status.refetch();
        }}
      />

      <SaveError error={action.error ?? saveError} />

      {!hasVulkan && (
        <p className={HINT}>
          No Vulkan driver was found, so games will use OpenGL and DirectX 12 titles will
          not start. Install your distribution's Vulkan driver for your graphics card.
        </p>
      )}
      <p className={HINT}>
        Only games running on Wine use these; Proton brings its own. The two go together:
        vkd3d-proton uses DXVK&rsquo;s DXGI, so DirectX 12 needs both. The version offered
        matches what your driver supports.
      </p>
    </Section>
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
    <Section title="Game fixes">
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
      <Check
        label="Apply per-title Proton fixes"
        hint="Passes each game's id to Proton so per-title workarounds apply. Matched by Steam AppID where your server knows one, by title otherwise."
        checked={settings.data?.umuFixes ?? true}
        onChange={(next) => save({ umuFixes: next })}
      />
      <Check
        label="Keep the list up to date automatically"
        hint="Fetches a fresh copy once a day, at startup and while Gameyfin keeps running."
        checked={settings.data?.umuAutoUpdate ?? true}
        onChange={(next) => save({ umuAutoUpdate: next })}
      />
      <div className="pt-1">
        <button
          type="button"
          disabled={action.busy}
          onClick={() => void refresh()}
          className={BUTTON_MAYBE_DISABLED}
        >
          {action.busy ? "Downloading…" : "Update now"}
        </button>
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
    <Section title="Compatibility">
      <label className="text-xs text-foreground/55" htmlFor="installer-memory">
        Installer memory limit
      </label>
      <select
        id="installer-memory"
        value={String(current)}
        onChange={(e) => void change(e.target.value)}
        className={INPUT}
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
      </select>
      <p className={HINT}>
        Repack installers take all the RAM they find, which can hang the whole machine.
        Automatic caps them at half your RAM, never below 4 GB, and covers the helpers they
        run. The cap counts address space rather than memory, so under 4 GB a 32-bit
        installer can fail with memory to spare. If one still gets stuck, try 3 GB.
      </p>
      <SaveError error={error} />
    </Section>
  );
}

/** Each game's prefix, removable one at a time. */
export function PrefixSection() {
  const action = useAction();
  const [confirming, setConfirming] = useState<PrefixEntry | null>(null);
  const prefixes = useQuery({ queryKey: keys.prefixes, queryFn: () => backend.listPrefixes() });

  function run(work: () => Promise<void>) {
    return action.run(async () => {
      await work();
      await prefixes.refetch();
    });
  }

  const rows = prefixes.data ?? [];

  return (
    <Section title="Compatibility prefixes">
      {rows.length === 0 ? (
        <p className="text-[11px] text-foreground/45">
          {prefixes.isLoading
            ? "…"
            : "None yet. One is created the first time a Windows game runs."}
        </p>
      ) : (
        <div className="flex flex-col gap-2">
          {rows.map((prefix) => (
            <div
              key={prefix.gameId}
              className="rounded-lg border border-default-200 bg-content2 p-2.5"
            >
              <div className="flex items-baseline justify-between gap-3">
                <span className="truncate text-xs text-foreground" title={prefix.path}>
                  {prefix.title ?? `Game ${prefix.gameId}`}
                </span>
                <span className="shrink-0 text-[11px] text-foreground/45">
                  {formatBytes(prefix.bytes)}
                </span>
              </div>
              {/* Called out because it is worth reclaiming. */}
              {!prefix.title && (
                <p className="mt-0.5 text-[11px] text-foreground/45">
                  No longer in your library.
                </p>
              )}
              <div className="mt-2 flex flex-wrap gap-1.5">
                <SmallButton onClick={() => void run(() => backend.openPrefixTool(prefix.gameId, "winecfg"))}>
                  Wine settings
                </SmallButton>
                <SmallButton onClick={() => void run(() => backend.openPrefixTool(prefix.gameId, "regedit"))}>
                  Registry
                </SmallButton>
                <SmallButton onClick={() => void run(() => backend.openPrefixTool(prefix.gameId, "explorer"))}>
                  Browse C:
                </SmallButton>
                <SmallButton danger onClick={() => setConfirming(prefix)}>
                  Delete
                </SmallButton>
              </div>
            </div>
          ))}
        </div>
      )}

      <SaveError error={action.error} />

      <p className={HINT}>
        A prefix is the fake Windows a game runs inside. A deleted one is rebuilt on the
        next launch, but anything the game stored inside it goes too.
      </p>

      {confirming && (
        <ConfirmDialog
          title={`Delete the prefix for ${confirming.title ?? `game ${confirming.gameId}`}?`}
          body={
            <>
              It is rebuilt the next time the game runs, so this recovers a broken one.
              Anything the game saved <em>inside</em> the prefix is removed with it, which
              for some Windows games includes save files.
            </>
          }
          confirmLabel="Delete the prefix"
          onConfirm={() => {
            const target = confirming;
            setConfirming(null);
            void run(() => backend.deletePrefix(target.gameId));
          }}
          onCancel={() => setConfirming(null)}
        />
      )}
    </Section>
  );
}
