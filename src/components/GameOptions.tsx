import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";

/**
 * Per-game launch options. Free-text boxes because the values come as a line of flags to
 * paste from a wiki or ProtonDB. Not a shell: quoting works, nothing else is interpreted
 * (see `arguments.rs`).
 */
export function GameOptions({ gameId }: { gameId: number }) {
  const options = useQuery({
    queryKey: ["game-options", gameId],
    queryFn: () => backend.gameOptions(gameId),
  });

  const [launch, setLaunch] = useState<string | null>(null);
  const [installer, setInstaller] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Reset drafts when the query lands or the game changes, so the boxes never show stale values.
  useEffect(() => {
    setLaunch(null);
    setInstaller(null);
  }, [gameId, options.data]);

  const currentLaunch = launch ?? options.data?.launchArguments ?? "";
  const currentInstaller = installer ?? options.data?.installerArguments ?? "";
  const dirty =
    (launch !== null && launch !== (options.data?.launchArguments ?? "")) ||
    (installer !== null && installer !== (options.data?.installerArguments ?? ""));

  async function save() {
    setError(null);
    try {
      await backend.setGameOptions(gameId, currentLaunch, currentInstaller);
      await options.refetch();
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } catch (e) {
      setError(messageOf(e));
    }
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
          htmlFor={`installer-args-${gameId}`}
        >
          Setup options
        </label>
        <input
          id={`installer-args-${gameId}`}
          value={currentInstaller}
          onChange={(e) => setInstaller(e.target.value)}
          spellCheck={false}
          placeholder="/VERYSILENT"
          className="w-full rounded-lg border border-default-200 bg-content2 px-2 py-1.5 font-mono text-[11px] outline-none focus:border-primary"
        />
        <p className="mt-1 text-[11px] leading-relaxed text-foreground/45">
          Passed to the game's setup program. A silent-install flag here is what lets an
          installer-based game be installed without the wizard asking anything.
        </p>
      </div>

      {error && (
        <p role="alert" className="text-[11px] leading-relaxed text-danger">
          {error}
        </p>
      )}

      {(dirty || saved) && (
        <div>
          <button
            type="button"
            onClick={() => void save()}
            className="rounded-lg bg-primary px-2.5 py-1 text-[11px] font-medium text-white transition-colors hover:bg-primary-600"
          >
            {saved ? "Saved" : "Save options"}
          </button>
        </div>
      )}
    </div>
  );
}
