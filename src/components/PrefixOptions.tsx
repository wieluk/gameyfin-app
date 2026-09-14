import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";

import { ConfirmDialog } from "@/components/ConfirmDialog";
import { backend } from "@/lib/backend";
import { formatBytes } from "@/lib/format";
import { isWindows } from "@/lib/platform";
import { keys } from "@/lib/queries";
import { useAction } from "@/lib/useAction";
import { HINT } from "@/lib/ui";
import { SmallButton } from "@/views/settings/controls";
import type { PrefixTool } from "@/bindings/PrefixTool";

/** Windows components worth a checkbox, under the name a player knows them by. */
const COMPONENTS: Array<{ verb: string; label: string }> = [
  { verb: "vcrun2022", label: "Visual C++ 2015 to 2022" },
  { verb: "vcrun2013", label: "Visual C++ 2013" },
  { verb: "vcrun2012", label: "Visual C++ 2012" },
  { verb: "vcrun2010", label: "Visual C++ 2010" },
  { verb: "vcrun2008", label: "Visual C++ 2008" },
  { verb: "d3dx9", label: "DirectX 9 extras" },
  { verb: "d3dx10", label: "DirectX 10 extras" },
  { verb: "d3dx11_43", label: "DirectX 11 extras" },
  { verb: "d3dcompiler_47", label: "Direct3D shader compiler" },
  { verb: "xact", label: "XAudio and XACT sound" },
  { verb: "xinput", label: "XInput controllers" },
  { verb: "dotnet48", label: ".NET Framework 4.8" },
  { verb: "xna40", label: "XNA Framework 4.0" },
  { verb: "physx", label: "PhysX" },
  { verb: "corefonts", label: "Microsoft core fonts" },
];

/** A Windows game's prefix tools and Winetricks, shown once its first launch created the prefix. */
export function PrefixOptions({ gameId, title }: { gameId: number; title: string }) {
  const queryClient = useQueryClient();
  const action = useAction();
  const [confirming, setConfirming] = useState(false);
  const [chosen, setChosen] = useState<Set<string> | null>(null);
  const [other, setOther] = useState("");
  const [result, setResult] = useState<string | null>(null);

  const prefixes = useQuery({
    queryKey: keys.prefixes,
    queryFn: () => backend.listPrefixes(),
    enabled: !isWindows,
  });
  const options = useQuery({
    queryKey: keys.gameOptions(gameId),
    queryFn: () => backend.gameOptions(gameId),
    enabled: !isWindows,
  });
  const suggested = options.data?.suggestedWinetricks ?? [];
  const suggestionKey = suggested.join(" ");

  // A new suggestion from a failed start replaces whatever was ticked before.
  useEffect(() => setChosen(null), [suggestionKey]);

  const prefix = prefixes.data?.find((entry) => entry.gameId === gameId);
  if (isWindows || !prefix) return null;

  const selected = chosen ?? new Set(suggested);
  // A suggestion the list does not name still gets its own box.
  const components = [
    ...COMPONENTS,
    ...suggested
      .filter((verb) => !COMPONENTS.some((component) => component.verb === verb))
      .map((verb) => ({ verb, label: verb })),
  ];
  const verbs = [...selected, ...other.split(/\s+/).filter(Boolean)];

  function toggle(verb: string) {
    const next = new Set(selected);
    if (next.has(verb)) next.delete(verb);
    else next.add(verb);
    setChosen(next);
  }

  async function refresh() {
    await Promise.all([
      prefixes.refetch(),
      queryClient.invalidateQueries({ queryKey: keys.gameOptions(gameId) }),
    ]);
  }

  async function install() {
    setResult(null);
    const message = await action.run(() => backend.runWinetricks(gameId, verbs.join(" ")));
    if (message !== undefined) {
      setResult(message);
      setChosen(new Set());
      setOther("");
    }
    await refresh();
  }

  function open(tool: PrefixTool) {
    void action.run(() => backend.openPrefixTool(gameId, tool));
  }

  return (
    <details className="rounded-lg border border-default-200/60 px-2.5 py-1.5">
      <summary className="cursor-pointer text-[11px] text-foreground/45">
        Compatibility prefix ({formatBytes(prefix.bytes)})
        {suggested.length > 0 && <span className="text-warning-600">, components missing</span>}
      </summary>

      <p className={`mt-2 ${HINT}`}>
        The small Windows this game runs inside. Only this game uses it.
      </p>
      <div className="mt-2 flex flex-wrap gap-1.5">
        <SmallButton onClick={() => open("winecfg")}>Wine settings</SmallButton>
        <SmallButton onClick={() => open("regedit")}>Registry</SmallButton>
        <SmallButton onClick={() => open("explorer")}>Browse C:</SmallButton>
        <SmallButton danger onClick={() => setConfirming(true)}>
          Delete prefix
        </SmallButton>
      </div>

      <p className="mt-3 text-[11px] font-medium text-foreground/70">Winetricks</p>
      <p className={HINT}>
        Installs Windows components a game needs but does not bring, such as Visual C++. Most
        games need none; use it when a start fails with a missing DLL or ProtonDB says to.
      </p>
      {suggested.length > 0 && (
        <p className="mt-1 text-[11px] leading-relaxed text-warning-600">
          The last start failed because components were missing. They are ticked below.
        </p>
      )}
      <div className="mt-1.5 grid grid-cols-1 gap-1 sm:grid-cols-2">
        {components.map((component) => (
          <label
            key={component.verb}
            title={component.verb}
            className="flex cursor-pointer items-center gap-2 text-[11px] text-foreground/80"
          >
            <input
              type="checkbox"
              checked={selected.has(component.verb)}
              disabled={action.busy}
              onChange={() => toggle(component.verb)}
              className="h-3.5 w-3.5 shrink-0 accent-primary"
            />
            {component.label}
          </label>
        ))}
      </div>
      <input
        aria-label="Other winetricks verbs"
        value={other}
        disabled={action.busy}
        onChange={(e) => setOther(e.target.value)}
        spellCheck={false}
        placeholder="Other verbs from ProtonDB, such as dotnet40"
        className="mt-1.5 w-full rounded-lg border border-default-200 bg-content2 px-2 py-1.5 font-mono text-[11px] outline-none focus:border-primary"
      />
      <div className="mt-1.5">
        <SmallButton disabled={action.busy || verbs.length === 0} onClick={() => void install()}>
          {action.busy
            ? "Installing, this can take minutes…"
            : verbs.length === 1
              ? "Install 1 component"
              : `Install ${verbs.length} components`}
        </SmallButton>
      </div>
      {result && <p className="mt-1 text-[11px] text-foreground/70">{result}</p>}
      {action.error && (
        <p role="alert" className="mt-1 text-[11px] leading-relaxed text-danger">
          {action.error}
        </p>
      )}

      {confirming && (
        <ConfirmDialog
          title={`Delete the prefix for ${title}?`}
          body={
            <>
              It is rebuilt the next time the game runs, so this recovers a broken one.
              Anything the game saved <em>inside</em> the prefix is removed with it, which
              for some Windows games includes save files.
            </>
          }
          confirmLabel="Delete the prefix"
          onConfirm={() => {
            setConfirming(false);
            void action.run(async () => {
              await backend.deletePrefix(gameId);
              await refresh();
            });
          }}
          onCancel={() => setConfirming(false)}
        />
      )}
    </details>
  );
}
