import { Icon } from "@/components/Icon";
import { Button, IconButton } from "@/components/ui";
import { BUTTON_HELP } from "@/lib/gamepad";
import { useCouch } from "@/state/couch";
import { useDismissOnEscape } from "@/lib/useDismiss";

/** The button map, shown by Start, since controller bindings are otherwise invisible. */
export function GamepadOverlay() {
  const helpOpen = useCouch((state) => state.helpOpen);

  if (!helpOpen) return null;
  return <Overlay />;
}

/** Its own component so the dismiss stack is only entered while the overlay is open. */
function Overlay() {
  const { closeHelp, name, couch, setCouch } = useCouch();
  // Through the shared stack, so only the topmost dialog closes.
  useDismissOnEscape(closeHelp);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6"
      onClick={closeHelp}
      role="presentation"
    >
      <div
        // Scoped so directional presses stay inside the overlay, not the library behind it.
        data-nav-scope
        role="dialog"
        aria-label="Controller buttons"
        className="w-full max-w-md rounded-2xl border border-default-200 bg-content1 p-5 shadow-xl"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="mb-4 flex items-center gap-2">
          <Icon name="controller" className="h-5 w-5 text-primary" />
          <h2 className="flex-1 text-sm font-semibold">Controller</h2>
          <IconButton icon="close" label="Close" size="sm" onClick={closeHelp} />
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

        <Button className="mt-4 w-full" onClick={() => setCouch(!couch)}>
          {couch ? "Switch back to the normal layout" : "Switch to the large layout"}
        </Button>
      </div>
    </div>
  );
}

/** A badge saying a controller is being read, so a pad that is not working can be told apart. */
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
