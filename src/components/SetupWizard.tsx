import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import { backend, isMockBackend } from "@/lib/backend";
import { useTauriEvent } from "@/lib/useTauriEvent";
import type { TransferProgress } from "@/bindings/TransferProgress";
import { formatBytes, formatSpeed } from "@/lib/format";
import { messageOf } from "@/lib/errors";
import { isWindows } from "@/lib/platform";
import { useAppSettings, useSettingsUpdate } from "@/lib/queries";
import { Modal } from "./Modal";
import { BUTTON_MAYBE_DISABLED } from "@/lib/ui";

type StepKey = "proton" | "wine" | "graphics" | "i386";

/** Something the wizard can download, with what it costs and whether it is already there. */
interface Step {
  key: StepKey;
  label: string;
  detail: string;
  bytes: number | null;
  installed: boolean;
  /** Why it cannot be installed here, when it cannot. */
  blocked: string | null;
  run: () => Promise<unknown>;
}

/**
 * First run on Linux: what this machine can do, and the pieces worth downloading now rather
 * than mid-launch. Re-opened from Settings by clearing the setting.
 */
export function SetupWizard() {
  const queryClient = useQueryClient();
  const settings = useAppSettings();
  const save = useSettingsUpdate();
  const [closed, setClosed] = useState(false);
  const [chosen, setChosen] = useState<Set<StepKey> | null>(null);
  const [busy, setBusy] = useState<StepKey | null>(null);
  const [progress, setProgress] = useState<TransferProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The runtime's extensions are mounted when the sandbox starts, so a fresh one is only
  // there after a restart.
  const [restartNeeded, setRestartNeeded] = useState(false);

  const done = settings.data?.setupDismissed ?? true;
  const enabled = !isWindows && !isMockBackend && settings.isSuccess && !done;

  const proton = useQuery({
    queryKey: ["proton-status"],
    queryFn: () => backend.protonStatus(),
    staleTime: 5 * 60 * 1000,
    enabled,
  });
  const wine = useQuery({
    queryKey: ["wine-status"],
    queryFn: () => backend.wineStatus(),
    staleTime: 5 * 60 * 1000,
    enabled,
  });
  const graphics = useQuery({
    queryKey: ["graphics-status"],
    queryFn: () => backend.graphicsStatus(),
    staleTime: 5 * 60 * 1000,
    enabled,
  });

  // While a step runs, whichever of the three is downloading reports on its own channel.
  const downloading = busy !== null;
  useTauriEvent<TransferProgress>("proton-progress", setProgress, downloading);
  useTauriEvent<TransferProgress>("wine-progress", setProgress, downloading);
  useTauriEvent<TransferProgress>("graphics-progress", setProgress, downloading);

  // Reopened from Settings, which clears the stored answer: without this the wizard stays
  // hidden until a restart.
  useEffect(() => {
    if (!done) setClosed(false);
  }, [done]);

  if (!enabled || closed) return null;

  const umuProblem = proton.data?.launcherProblem ?? null;
  const hasProton = proton.data?.installed.some((build) => build.source === "managed") ?? false;
  const hasWine = Boolean(wine.data?.installed);
  const hasGraphics = Boolean(graphics.data?.installed.dxvk && graphics.data?.installed.vkd3d);
  const noVulkan = graphics.data ? graphics.data.vulkan.apiVersion === null : false;
  const no32Bit = proton.data ? !proton.data.supports32bit : false;
  // In a Flatpak the 32-bit libraries are one download away; elsewhere they are the
  // distribution's to install, so there is nothing to offer.
  const canAdd32Bit = proton.data?.missingI386Extension ?? false;

  const steps: Step[] = [
    {
      key: "proton",
      label: "Proton",
      detail: "Runs Windows games the way Steam does, in Valve's Steam Runtime.",
      bytes: proton.data?.latestUmu?.sizeBytes ?? null,
      installed: hasProton,
      blocked: umuProblem,
      run: () => backend.installProton("umu-proton"),
    },
    {
      key: "wine",
      label: "Wine",
      detail: no32Bit
        ? "Needed here: this system has no 32-bit libraries, so 32-bit games and installers run on Wine."
        : "The fallback, for a game that misbehaves in the container.",
      bytes: wine.data?.latest?.sizeBytes ?? null,
      installed: hasWine,
      blocked: null,
      run: () => backend.installWine(),
    },
    {
      key: "graphics",
      label: "Direct3D for Wine",
      detail: "DXVK and vkd3d-proton, which Wine needs to draw. Proton brings its own.",
      bytes:
        (graphics.data?.latestDxvk?.sizeBytes ?? 0) + (graphics.data?.latestVkd3d?.sizeBytes ?? 0) ||
        null,
      installed: hasGraphics,
      blocked: noVulkan ? "No Vulkan driver was found, so these cannot be used." : null,
      run: async () => {
        await backend.installGraphics("dxvk");
        await backend.installGraphics("vkd3d");
      },
    },
    // Only where it can be fixed from here. It comes from Flathub rather than from
    // Gameyfin, which is why it is not installed along with the app.
    ...(canAdd32Bit
      ? [
          {
            key: "i386" as const,
            label: "32-bit support",
            detail:
              "The Flatpak runtime's 32-bit libraries, from Flathub. Installers and older games need them to run in the container rather than on Wine.",
            bytes: null,
            installed: false,
            blocked: null,
            run: async () => {
              await backend.install32bitSupport();
              setRestartNeeded(true);
            },
          },
        ]
      : []),
  ];

  // Everything that can be downloaded starts ticked, so the usual answer is one click.
  const suggested = new Set(
    steps.filter((step) => !step.installed && !step.blocked).map((step) => step.key),
  );
  const selection = chosen ?? suggested;

  function toggle(key: StepKey) {
    const next = new Set(selection);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    setChosen(next);
  }

  async function finish() {
    setClosed(true);
    try {
      await save({ setupDismissed: true });
    } catch {
      // The wizard is already gone for this run; a failed write only means it returns.
    }
  }

  async function install() {
    setError(null);
    for (const step of steps) {
      if (!selection.has(step.key) || step.installed || step.blocked) continue;
      setBusy(step.key);
      setProgress(null);
      try {
        await step.run();
      } catch (e) {
        setError(`${step.label}: ${messageOf(e)}`);
        setBusy(null);
        return;
      }
    }
    setBusy(null);
    setProgress(null);
    await Promise.all([proton.refetch(), wine.refetch(), graphics.refetch()]);
    // The install screen greys out without a runtime; tell it one exists now.
    await queryClient.invalidateQueries({ queryKey: ["install-options"] });
    // Left open when a restart is what makes the last step count, so the user reads why.
    if (!restartNeeded) await finish();
  }

  const nothingToDo = steps.every((step) => step.installed || step.blocked);
  const loading = proton.isLoading || wine.isLoading || graphics.isLoading;

  return (
    <Modal
      label="Set up Windows games"
      onDismiss={() => !busy && setClosed(true)}
      dismissOnBackdrop={false}
    >
      <div className="px-5 py-4">
        <h2 className="mb-1 text-sm font-semibold text-foreground">
          Set up Windows games
        </h2>
        <p className="text-xs leading-relaxed text-foreground/60">
          Gameyfin keeps its own copies, so nothing is installed on your system and no admin
          rights are needed. Skip this if your library is native Linux games; anything missing
          is downloaded when a game first needs it.
        </p>

        <dl className="mt-3 flex flex-col gap-1 rounded-lg border border-default-200 bg-content2 px-2.5 py-2">
          <Finding
            label="Proton (umu)"
            value={loading ? "…" : umuProblem ? "Cannot run here" : "Ready"}
            good={!umuProblem}
            note={umuProblem ?? undefined}
          />
          <Finding
            label="32-bit programs"
            value={loading ? "…" : no32Bit ? "Not supported" : "Supported"}
            good={!no32Bit}
            note={
              !no32Bit
                ? undefined
                : canAdd32Bit
                  ? "Tick 32-bit support below to install them; without them, installers and 32-bit games run on Wine instead of Proton."
                  : "Installers and 32-bit games will run on Wine instead of Proton."
            }
          />
          <Finding
            label="Vulkan"
            value={loading ? "…" : (graphics.data?.vulkanLabel ?? "Unknown")}
            good={!noVulkan}
            note={noVulkan ? "Games will fall back to OpenGL and run slowly." : undefined}
          />
        </dl>

        <div className="mt-3 flex flex-col gap-1.5">
          {steps.map((step) => (
            <label
              key={step.key}
              className={`flex items-start gap-2 rounded-lg border px-2.5 py-2 text-xs ${
                step.installed || step.blocked
                  ? "border-default-200/60 opacity-60"
                  : "border-default-200"
              }`}
            >
              <input
                type="checkbox"
                className="mt-0.5 h-3.5 w-3.5 accent-primary"
                checked={selection.has(step.key)}
                disabled={Boolean(busy) || step.installed || Boolean(step.blocked)}
                onChange={() => toggle(step.key)}
              />
              <span className="min-w-0 flex-1">
                <span className="flex items-baseline justify-between gap-2">
                  <span className="font-medium text-foreground">{step.label}</span>
                  <span className="shrink-0 text-[11px] text-foreground/45">
                    {step.installed
                      ? "Installed"
                      : step.bytes
                        ? formatBytes(step.bytes)
                        : ""}
                  </span>
                </span>
                <span className="mt-0.5 block leading-relaxed text-foreground/55">
                  {step.blocked ?? step.detail}
                </span>
              </span>
            </label>
          ))}
        </div>

        {busy && (
          <div className="mt-3 flex flex-col gap-1">
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

        {restartNeeded && !busy && (
          <p role="status" className="mt-3 text-[11px] leading-relaxed text-success-600">
            The 32-bit libraries are installed. Restart Gameyfin to use them.
          </p>
        )}

        {error && (
          <p role="alert" className="mt-3 text-[11px] leading-relaxed text-danger">
            {error}
          </p>
        )}
      </div>

      <div className="flex justify-end gap-2 border-t border-default-200/60 px-5 py-3">
        <button
          type="button"
          disabled={Boolean(busy)}
          onClick={() => void finish()}
          className={BUTTON_MAYBE_DISABLED}
        >
          {nothingToDo ? "Close" : "Skip"}
        </button>
        {!nothingToDo && (
          <button
            type="button"
            autoFocus
            disabled={Boolean(busy) || selection.size === 0}
            onClick={() => void install()}
            className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground transition-colors hover:bg-primary-600 disabled:opacity-50"
          >
            {busy ? "Downloading…" : "Download selected"}
          </button>
        )}
      </div>
    </Modal>
  );
}

/** One line of what the wizard found, with the consequence when it is bad news. */
function Finding({
  label,
  value,
  good,
  note,
}: {
  label: string;
  value: string;
  good: boolean;
  note?: string;
}) {
  return (
    <div className="flex flex-col">
      <div className="flex items-baseline justify-between gap-3 text-xs">
        <dt className="text-foreground/55">{label}</dt>
        <dd className={good ? "text-success" : "text-danger"}>{value}</dd>
      </div>
      {note && <p className="text-[11px] leading-relaxed text-foreground/45">{note}</p>}
    </div>
  );
}
