import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import { isWindows } from "@/lib/platform";
import { keys } from "@/lib/queries";
import { useFlash } from "@/lib/useFlash";
import type { GameOptionsPatch } from "@/bindings/GameOptionsPatch";
import type { LaunchToggles } from "@/bindings/LaunchToggles";

const NO_TOGGLES: LaunchToggles = { wayland: false, wow64: false };

/**
 * Per-game launch and setup options, as free text pasted from a wiki or ProtonDB. Not a shell:
 * quoting works, nothing else is interpreted (see `arguments.rs`).
 */

/** The stored options, and a save that leaves the fields it is not given alone. */
function useGameOptions(gameId: number) {
  const options = useQuery({
    queryKey: keys.gameOptions(gameId),
    queryFn: () => backend.gameOptions(gameId),
  });

  // A patch, so the two option boxes on one row cannot write each other's stale value back.
  async function save(change: GameOptionsPatch) {
    await backend.setGameOptions(gameId, change);
    await options.refetch();
  }

  return { data: options.data, save };
}

/** Everything that applies to running the game: flags, environment and Proton settings. */
export function LaunchOptions({ gameId }: { gameId: number }) {
  const { data, save } = useGameOptions(gameId);

  const [launch, setLaunch] = useState<string | null>(null);
  const [environment, setEnvironment] = useState<string | null>(null);
  // Shown at once while the save runs, since switches and pickers save on change.
  const [protonBuild, setProtonBuild] = useState<string | null>(null);
  const [toggles, setToggles] = useState<LaunchToggles | null>(null);
  const [saved, flashSaved] = useFlash();
  const [error, setError] = useState<string | null>(null);

  // Reset drafts when the query lands or the game changes, so the boxes never show stale values.
  useEffect(() => {
    setLaunch(null);
    setEnvironment(null);
    setProtonBuild(null);
    setToggles(null);
  }, [gameId, data]);

  const currentLaunch = launch ?? data?.launchArguments ?? "";
  const currentEnvironment = environment ?? data?.launchEnvironment ?? "";
  const currentProtonBuild = protonBuild ?? data?.protonBuild ?? "";
  const currentToggles = toggles ?? data?.launchToggles ?? NO_TOGGLES;
  // Only the text boxes wait for Save: a half-typed flag must not reach the next launch.
  const dirty =
    (launch !== null && launch !== (data?.launchArguments ?? "")) ||
    (environment !== null && environment !== (data?.launchEnvironment ?? ""));

  async function apply(change: GameOptionsPatch) {
    setError(null);
    try {
      await save(change);
      flashSaved();
    } catch (e) {
      setError(messageOf(e));
      setProtonBuild(null);
      setToggles(null);
    }
  }

  function changeBuild(build: string) {
    setProtonBuild(build);
    // Empty clears the pin.
    void apply({ protonBuild: build });
  }

  function changeToggles(next: LaunchToggles) {
    setToggles(next);
    void apply({ launchToggles: next });
  }

  function commit() {
    void apply({ launchArguments: currentLaunch, launchEnvironment: currentEnvironment });
  }

  return (
    <div className="flex flex-col gap-2">
      <div>
        <label
          className="mb-1 block text-[11px] text-foreground/45"
          htmlFor={`launch-args-${gameId}`}
        >
          Launch options
        </label>
        <input
          id={`launch-args-${gameId}`}
          value={currentLaunch}
          onChange={(e) => setLaunch(e.target.value)}
          spellCheck={false}
          placeholder="-windowed -nolauncher"
          className="w-full rounded-lg border border-default-200 bg-content2 px-2 py-1.5 font-mono text-[11px] outline-none focus:border-primary"
        />
      </div>

      <div>
        <label
          className="mb-1 block text-[11px] text-foreground/45"
          htmlFor={`launch-env-${gameId}`}
        >
          Environment variables
        </label>
        <textarea
          id={`launch-env-${gameId}`}
          value={currentEnvironment}
          onChange={(e) => setEnvironment(e.target.value)}
          spellCheck={false}
          rows={3}
          placeholder="DXVK_HUD=fps"
          className="w-full resize-y rounded-lg border border-default-200 bg-content2 px-2 py-1.5 font-mono text-[11px] outline-none focus:border-primary"
        />
        <p className="mt-1 text-[11px] leading-relaxed text-foreground/45">
          One KEY=value per line, set when the game starts. This is where a fix copied from
          ProtonDB goes.
        </p>
      </div>

      {!isWindows && (data?.protonBuilds.length ?? 0) > 1 && (
        <div>
          <label
            className="mb-1 block text-[11px] text-foreground/45"
            htmlFor={`proton-build-${gameId}`}
          >
            Proton build
          </label>
          <select
            id={`proton-build-${gameId}`}
            value={currentProtonBuild}
            onChange={(e) => changeBuild(e.target.value)}
            className="w-full rounded-lg border border-default-200 bg-content2 px-2 py-1.5 text-[11px] outline-none focus:border-primary"
          >
            <option value="">UMU-Proton (default)</option>
            {data?.protonBuilds
              .filter((build) => build.family !== "umu-proton")
              .map((build) => (
                <option key={build.name} value={build.name}>
                  {build.name}
                </option>
              ))}
          </select>
        </div>
      )}

      {!isWindows && (
        <div className="flex flex-col gap-1.5">
          <ProtonSwitch
            label="Wayland"
            hint="Draws the game without XWayland."
            checked={currentToggles.wayland}
            onChange={(wayland) => changeToggles({ ...currentToggles, wayland })}
          />
          <ProtonSwitch
            label="WOW64"
            hint="Runs 32-bit games without 32-bit system libraries. Can break anti-cheat."
            checked={currentToggles.wow64}
            onChange={(wow64) => changeToggles({ ...currentToggles, wow64 })}
          />
        </div>
      )}

      {error && (
        <p role="alert" className="text-[11px] leading-relaxed text-danger">
          {error}
        </p>
      )}

      {dirty ? (
        <div>
          <button
            type="button"
            onClick={commit}
            className="rounded-lg bg-primary px-2.5 py-1 text-[11px] font-medium text-white transition-colors hover:bg-primary-600"
          >
            Save options
          </button>
        </div>
      ) : (
        saved && <p className="text-[11px] text-success-600">Saved</p>
      )}
    </div>
  );
}

function ProtonSwitch({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint: string;
  checked: boolean;
  onChange: (next: boolean) => void;
}) {
  return (
    <label className="flex cursor-pointer items-start gap-2 text-[11px] text-foreground/80">
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
        className="mt-0.5 h-3.5 w-3.5 shrink-0 accent-primary"
      />
      <span>
        {label}
        <span className="block leading-relaxed text-foreground/45">{hint}</span>
      </span>
    </label>
  );
}

/** Flags for the game's setup program, for an install that has yet to run. */
export function SetupOptions({ gameId, hint }: { gameId: number; hint?: string }) {
  const { data, save } = useGameOptions(gameId);
  const stored = data?.installerArguments ?? "";

  const [draft, setDraft] = useState<string | null>(null);
  const [saved, flashSaved] = useFlash();
  const [error, setError] = useState<string | null>(null);

  // The draft drops whenever the stored value changes, so the box never shows a stale edit.
  useEffect(() => setDraft(null), [stored]);

  const current = draft ?? stored;
  const dirty = draft !== null && draft !== stored;

  async function commit() {
    setError(null);
    try {
      await save({ installerArguments: current });
      flashSaved();
    } catch (e) {
      setError(messageOf(e));
    }
  }

  return (
    <div>
      <label
        className="mb-1 block text-[11px] text-foreground/45"
        htmlFor={`installer-args-${gameId}`}
      >
        Setup options
      </label>
      <input
        id={`installer-args-${gameId}`}
        value={current}
        onChange={(e) => setDraft(e.target.value)}
        spellCheck={false}
        placeholder="/VERYSILENT"
        className="w-full rounded-lg border border-default-200 bg-content2 px-2 py-1.5 font-mono text-[11px] outline-none focus:border-primary"
      />
      {hint && <p className="mt-1 text-[11px] leading-relaxed text-foreground/45">{hint}</p>}

      {error && (
        <p role="alert" className="mt-1 text-[11px] leading-relaxed text-danger">
          {error}
        </p>
      )}

      {(dirty || saved) && (
        <button
          type="button"
          onClick={() => void commit()}
          className="mt-1.5 rounded-lg bg-primary px-2.5 py-1 text-[11px] font-medium text-white transition-colors hover:bg-primary-600"
        >
          {saved ? "Saved" : "Save"}
        </button>
      )}
    </div>
  );
}
