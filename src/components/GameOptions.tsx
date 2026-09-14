import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Alert } from "@/components/Alert";
import { FormField, Select, SwitchField, TextArea, TextInput } from "@/components/ui";
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

/** Pressing Enter in a single-line box leaves it, which is what saves it. */
function blurOnEnter(e: React.KeyboardEvent<HTMLInputElement>) {
  if (e.key === "Enter") e.currentTarget.blur();
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

  // Each draft drops only when its own value lands, so saving one box never wipes another mid-edit.
  useEffect(() => setLaunch(null), [gameId, data?.launchArguments]);
  useEffect(() => setEnvironment(null), [gameId, data?.launchEnvironment]);
  useEffect(() => setProtonBuild(null), [gameId, data?.protonBuild]);
  useEffect(
    () => setToggles(null),
    [gameId, data?.launchToggles?.wayland, data?.launchToggles?.wow64],
  );

  const storedLaunch = data?.launchArguments ?? "";
  const storedEnvironment = data?.launchEnvironment ?? "";
  const currentLaunch = launch ?? storedLaunch;
  const currentEnvironment = environment ?? storedEnvironment;
  const currentProtonBuild = protonBuild ?? data?.protonBuild ?? "";
  const currentToggles = toggles ?? data?.launchToggles ?? NO_TOGGLES;

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

  // Text saves on leaving the box, so a half-typed flag never reaches a launch.
  function commitLaunch() {
    if (launch !== null && launch !== storedLaunch) void apply({ launchArguments: launch });
  }

  function commitEnvironment() {
    if (environment !== null && environment !== storedEnvironment) {
      void apply({ launchEnvironment: environment });
    }
  }

  return (
    <div className="flex flex-col gap-2">
      <FormField label="Launch options" htmlFor={`launch-args-${gameId}`}>
        <TextInput
          id={`launch-args-${gameId}`}
          mono
          value={currentLaunch}
          onChange={(e) => setLaunch(e.target.value)}
          onBlur={commitLaunch}
          onKeyDown={blurOnEnter}
          spellCheck={false}
          placeholder="-windowed -nolauncher"
        />
      </FormField>

      <FormField
        label="Environment variables"
        htmlFor={`launch-env-${gameId}`}
        hint="One KEY=value per line, set when the game starts. This is where a fix copied from ProtonDB goes."
      >
        <TextArea
          id={`launch-env-${gameId}`}
          mono
          rows={3}
          value={currentEnvironment}
          onChange={(e) => setEnvironment(e.target.value)}
          onBlur={commitEnvironment}
          spellCheck={false}
          placeholder="DXVK_HUD=fps"
        />
      </FormField>

      {!isWindows && (data?.protonBuilds.length ?? 0) > 1 && (
        <FormField label="Proton build" htmlFor={`proton-build-${gameId}`}>
          <Select
            id={`proton-build-${gameId}`}
            value={currentProtonBuild}
            onChange={(e) => changeBuild(e.target.value)}
          >
            <option value="">UMU-Proton (default)</option>
            {data?.protonBuilds
              .filter((build) => build.family !== "umu-proton")
              .map((build) => (
                <option key={build.name} value={build.name}>
                  {build.name}
                </option>
              ))}
          </Select>
        </FormField>
      )}

      {!isWindows && (
        <div className="flex flex-col gap-2 pt-1">
          <SwitchField
            label="Wayland"
            hint="Draws the game without XWayland."
            checked={currentToggles.wayland}
            onChange={(wayland) => changeToggles({ ...currentToggles, wayland })}
          />
          <SwitchField
            label="WOW64"
            hint="Runs 32-bit games without 32-bit system libraries. Can break anti-cheat."
            checked={currentToggles.wow64}
            onChange={(wow64) => changeToggles({ ...currentToggles, wow64 })}
          />
        </div>
      )}

      {error && <Alert inline>{error}</Alert>}
      {saved && <p className="text-[11px] text-success-600">Saved</p>}
    </div>
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

  async function commit() {
    if (draft === null || draft === stored) return;
    setError(null);
    try {
      await save({ installerArguments: draft });
      flashSaved();
    } catch (e) {
      setError(messageOf(e));
    }
  }

  return (
    <div className="flex flex-col gap-1">
      <FormField label="Setup options" htmlFor={`installer-args-${gameId}`} hint={hint}>
        <TextInput
          id={`installer-args-${gameId}`}
          mono
          value={current}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={() => void commit()}
          onKeyDown={blurOnEnter}
          spellCheck={false}
          placeholder="/VERYSILENT"
        />
      </FormField>
      {error && <Alert inline>{error}</Alert>}
      {saved && <p className="text-[11px] text-success-600">Saved</p>}
    </div>
  );
}
