import { useEffect } from "react";

import { Icon } from "@/components/Icon";
import { BUTTON_HELP } from "@/lib/gamepad";
import { useCouch } from "@/state/couch";

/**
 * The button map, shown by Start.
 *
 * Discoverability is the whole reason this exists. Controller bindings are invisible, and
 * an app whose bindings can only be learned from a README is one most people will drive
 * with a mouse instead.
 */
export function GamepadOverlay() {
  const { helpOpen, closeHelp, name, couch, setCouch } = useCouch();

  useEffect(() => {
    if (!helpOpen) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") closeHelp();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [helpOpen, closeHelp]);

  if (!helpOpen) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6"
      onClick={closeHelp}
      role="presentation"
    >
      <div
        // Scoped so directional presses stay inside the overlay rather than moving focus
        // through the library behind it.
        data-nav-scope
        role="dialog"
        aria-label="Controller buttons"
        className="w-full max-w-md rounded-2xl border border-default-200 bg-content1 p-5 shadow-xl"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="mb-4 flex items-center gap-2">
          <Icon name="controller" className="h-5 w-5 text-primary" />
          <h2 className="flex-1 text-sm font-semibold">Controller</h2>
          <button
            type="button"
            onClick={closeHelp}
            aria-label="Close"
            className="flex h-7 w-7 items-center justify-center rounded-lg text-foreground/45 transition-colors hover:bg-default-100 hover:text-foreground"
          >
            <Icon name="close" className="h-4 w-4" />
          </button>
        </div>

        {name && (
          <p className="mb-3 truncate text-[11px] text-foreground/45" title={name}>
            Connected: {name}
          </p>
        )}

        <dl className="flex flex-col gap-1.5">
          {BUTTON_HELP.map((row) => (
            <div key={row.keys} className="flex items-baseline justify-between gap-4 text-xs">
              <dt className="shrink-0 rounded border border-default-200 bg-content2 px-1.5 py-0.5 font-mono text-[10px] text-foreground/70">
                {row.keys}
              </dt>
              <dd className="text-right text-foreground/60">{row.action}</dd>
            </div>
          ))}
        </dl>

        <button
          type="button"
          onClick={() => setCouch(!couch)}
          className="mt-4 w-full rounded-lg border border-default-200 px-3 py-2 text-xs text-foreground/70 transition-colors hover:bg-default-100"
        >
          {couch ? "Switch back to the normal layout" : "Switch to the large layout"}
        </button>
      </div>
    </div>
  );
}

/**
 * A small badge saying a controller is being read.
 *
 * Without it, a pad that is connected but not working looks identical to one the app has
 * not noticed, and there is no way to tell which from inside the app.
 */
export function GamepadIndicator() {
  const { connected, toggleHelp } = useCouch();
  if (!connected) return null;

  return (
    <button
      type="button"
      onClick={toggleHelp}
      title="Controller connected. Press Start for the button map."
      aria-label="Controller connected, show the button map"
      className="flex items-center gap-1 rounded-lg px-2 py-1 text-[11px] text-foreground/45 transition-colors hover:bg-default-100 hover:text-foreground"
    >
      <Icon name="controller" className="h-3.5 w-3.5" />
    </button>
  );
}
