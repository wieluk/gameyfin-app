import { useState } from "react";

import { backend } from "@/lib/backend";
import { useAppSettings } from "@/lib/queries";
import { BUTTON, HINT, INPUT } from "@/lib/ui";
import { useCouch } from "@/state/couch";
import type { Theme } from "@/types";
import { Check, Row, SaveError, Section, useSettingSaver } from "./controls";

export function AppearanceSection() {
  const settings = useAppSettings();
  const { save, error } = useSettingSaver();

  return (
    <Section title="Appearance">
      <label className="text-xs text-foreground/55" htmlFor="theme">
        Theme
      </label>
      <select
        id="theme"
        value={settings.data?.theme ?? "dark"}
        onChange={(e) => void save({ theme: e.target.value as Theme })}
        className={INPUT}
      >
        <option value="dark">Dark</option>
        <option value="light">Light</option>
        <option value="system">Match my system</option>
      </select>
      <SaveError error={error} />
    </Section>
  );
}

export function NotificationSection() {
  const settings = useAppSettings();
  const { save, error } = useSettingSaver();

  return (
    <Section title="Notifications">
      <Check
        label="Downloads and installs"
        hint="When a download is ready to install, and when a game is ready to play."
        checked={settings.data?.notifyTransfers ?? true}
        onChange={(next) => save({ notifyTransfers: next })}
      />
      <Check
        label="Failures"
        hint="When a download, install or launch goes wrong. Shown even while you are looking at the window."
        checked={settings.data?.notifyFailures ?? true}
        onChange={(next) => save({ notifyFailures: next })}
      />
      <Check
        label="New versions of Gameyfin"
        checked={settings.data?.notifyUpdates ?? true}
        onChange={(next) => save({ notifyUpdates: next })}
      />
      <p className={HINT}>Held back while you are looking at the window, apart from failures.</p>
      <SaveError error={error} />
    </Section>
  );
}

/** The tray, and what the close button does. */
export function WindowSection() {
  const settings = useAppSettings();
  const { save, error } = useSettingSaver();

  return (
    <Section title="Window">
      <Check
        label="Closing the window keeps Gameyfin running"
        hint="Downloads run inside this program, so closing the window stops one. Turn this on and the close button hides the window instead, with the tray icon to bring it back."
        checked={settings.data?.closeToTray ?? false}
        onChange={(next) => save({ closeToTray: next })}
      />
      <Check
        label="Start hidden in the tray"
        hint="For starting Gameyfin with your session without a window appearing."
        checked={settings.data?.startMinimized ?? false}
        onChange={(next) => save({ startMinimized: next })}
      />
      <Check
        label="Start Gameyfin when I log in"
        hint="Starts Gameyfin hidden in the tray with your session, so background downloads keep working."
        checked={settings.data?.autostart ?? false}
        onChange={(next) => save({ autostart: next })}
      />
      <SaveError error={error} />
      <div className="pt-1">
        <button
          type="button"
          onClick={() => void backend.quitApp()}
          className="rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/70 transition-colors hover:border-danger/40 hover:bg-danger/10 hover:text-danger"
        >
          Quit Gameyfin
        </button>
      </div>
    </Section>
  );
}

export function GamepadSection() {
  const settings = useAppSettings();
  const connected = useCouch((state) => state.connected);
  const name = useCouch((state) => state.name);
  const toggleHelp = useCouch((state) => state.toggleHelp);
  const { save, error } = useSettingSaver();

  const stored = settings.data?.gamepadDeadzone ?? 0.25;
  // Dragging is local: saving per step would refetch under the thumb and snap it back.
  const [dragged, setDragged] = useState<number | null>(null);
  const deadzone = dragged ?? stored;

  return (
    <Section title="Controller">
      <Row
        label="Detected"
        value={connected ? (name ?? "A controller") : "None connected"}
        tone={connected ? "good" : undefined}
      />
      <Check
        label="Read connected controllers"
        checked={settings.data?.gamepadEnabled ?? true}
        onChange={(next) => save({ gamepadEnabled: next })}
      />
      <Check
        label="Switch to the large layout when a controller connects"
        hint="Bigger text and larger covers, for reading from a sofa. Switch back from the controller overlay any time."
        checked={settings.data?.couchModeAuto ?? true}
        onChange={(next) => save({ couchModeAuto: next })}
      />

      <label className="pt-1 text-xs text-foreground/55" htmlFor="gamepad-deadzone">
        Stick dead zone: {Math.round(deadzone * 100)}%
      </label>
      <input
        id="gamepad-deadzone"
        type="range"
        min={5}
        max={60}
        step={5}
        value={Math.round(deadzone * 100)}
        onChange={(e) => setDragged(Number(e.target.value) / 100)}
        onPointerUp={() => commit()}
        onKeyUp={() => commit()}
        onBlur={() => commit()}
        className="w-full accent-primary"
      />
      <p className={HINT}>
        How far a stick must move before it counts. Raise it if the selection drifts on its
        own.
      </p>
      <SaveError error={error} />

      <div className="pt-1">
        <button type="button" onClick={toggleHelp} className={BUTTON}>
          Show the button map
        </button>
      </div>
    </Section>
  );

  function commit() {
    if (dragged === null || dragged === stored) return;
    void save({ gamepadDeadzone: dragged }).then(() => setDragged(null));
  }
}
