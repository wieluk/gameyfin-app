import { useState } from "react";

import { Button, FormField, Select, SwitchField } from "@/components/ui";
import { backend } from "@/lib/backend";
import { useAppSettings } from "@/lib/queries";
import { HINT } from "@/lib/ui";
import { useCouch } from "@/state/couch";
import type { Theme } from "@/types";
import { Row, SaveError, Section, useSettingSaver } from "./controls";

export function AppearanceSection() {
  const settings = useAppSettings();
  const { save, error } = useSettingSaver();

  return (
    <Section title="Appearance">
      <FormField label="Theme" htmlFor="theme">
        <Select
          id="theme"
          value={settings.data?.theme ?? "dark"}
          onChange={(e) => void save({ theme: e.target.value as Theme })}
        >
          <option value="dark">Dark</option>
          <option value="light">Light</option>
          <option value="system">Match my system</option>
        </Select>
      </FormField>
      <SaveError error={error} />
    </Section>
  );
}

export function NotificationSection() {
  const settings = useAppSettings();
  const { save, error } = useSettingSaver();
  const enabled = settings.data?.notifyEnabled ?? true;

  // Each kind, in the order they are listed under the main switch.
  const kinds: Array<{ key: NotifyKey; label: string; hint?: string }> = [
    {
      key: "notifyTransfers",
      label: "Downloads and installs",
      hint: "When a download is ready to install, and when a game is ready to play.",
    },
    {
      key: "notifyFailures",
      label: "Failures",
      hint: "When a download, install or launch goes wrong. Shown even while you are looking at the window.",
    },
    { key: "notifyUpdates", label: "New versions of Gameyfin" },
    {
      key: "notifyProton",
      label: "New Proton versions",
      hint: "For the Proton builds you have downloaded.",
    },
    {
      key: "notifySignIn",
      label: "Signed out",
      hint: "When the server stops accepting your sign-in. Shown even while you are looking at the window.",
    },
    {
      key: "notifyConnection",
      label: "Server connection",
      hint: "When the server stops answering, and when it is back.",
    },
    {
      key: "notifySaves",
      label: "Save sync",
      hint: "When a save cannot sync, or a save needs your choice.",
    },
    { key: "notifyNewGames", label: "New games", hint: "When games are added to the server." },
    {
      key: "notifyGameUpdates",
      label: "Updated games",
      hint: "When an installed game has new files on the server.",
    },
  ];

  return (
    <Section title="Notifications" help="notifications">
      <SwitchField
        label="Show notifications"
        hint="Turn off to keep them only in the list behind the bell."
        checked={enabled}
        onChange={(next) => save({ notifyEnabled: next })}
      >
        {kinds.map((kind) => (
          <SwitchField
            key={kind.key}
            label={kind.label}
            hint={kind.hint}
            checked={settings.data?.[kind.key] ?? true}
            disabled={!enabled}
            onChange={(next) => save({ [kind.key]: next })}
          />
        ))}
      </SwitchField>
      <p className={`${HINT} mt-1 border-t border-default-200/60 pt-2`}>
        Held back while you are looking at the window, apart from failures and sign-outs. Every
        notification is also listed behind the bell in the title bar.
      </p>
      <SaveError error={error} />
    </Section>
  );
}

/** The per-kind notification switches, which all share the main one. */
type NotifyKey =
  | "notifyTransfers"
  | "notifyFailures"
  | "notifyUpdates"
  | "notifyProton"
  | "notifySignIn"
  | "notifyConnection"
  | "notifySaves"
  | "notifyNewGames"
  | "notifyGameUpdates";

/** The tray, and what the close button does. */
export function WindowSection() {
  const settings = useAppSettings();
  const { save, error } = useSettingSaver();
  const autostart = settings.data?.autostart ?? false;

  return (
    <Section title="Window" help="window">
      <SwitchField
        label="Closing the window keeps Gameyfin running"
        hint="Downloads run inside this program, so closing the window stops one. Turn this on and the close button hides the window instead, with the tray icon to bring it back."
        checked={settings.data?.closeToTray ?? false}
        onChange={(next) => save({ closeToTray: next })}
      />
      <SwitchField
        label="Start Gameyfin when I log in"
        hint="Starts Gameyfin with your session, so background downloads keep working."
        checked={autostart}
        onChange={(next) => save({ autostart: next })}
      >
        <SwitchField
          label="Start hidden in the tray"
          hint="Only for that start. Opening Gameyfin yourself always shows the window."
          checked={settings.data?.autostartHidden ?? true}
          disabled={!autostart}
          onChange={(next) => save({ autostartHidden: next })}
        />
      </SwitchField>
      <SwitchField
        label="Bring Gameyfin back when a game closes"
        hint="Raises the window once a game quits, where its save upload and any error are shown. Turn it off if you start games from Steam."
        checked={settings.data?.focusAfterGame ?? true}
        onChange={(next) => save({ focusAfterGame: next })}
      />
      <SaveError error={error} />
      <div className="pt-1">
        <Button variant="destructive" onClick={() => void backend.quitApp()}>
          Quit Gameyfin
        </Button>
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
    <Section title="Controller" help="controllers">
      <Row
        label="Detected"
        value={connected ? (name ?? "A controller") : "None connected"}
        tone={connected ? "good" : undefined}
      />
      <SwitchField
        label="Read connected controllers"
        checked={settings.data?.gamepadEnabled ?? true}
        onChange={(next) => save({ gamepadEnabled: next })}
      />
      <SwitchField
        label="Switch to the large layout when a controller connects"
        hint="Bigger text and larger covers, for reading from a sofa. Switch back from the controller overlay any time."
        checked={settings.data?.couchModeAuto ?? true}
        onChange={(next) => save({ couchModeAuto: next })}
      />

      <FormField
        className="pt-1"
        label={`Stick dead zone: ${Math.round(deadzone * 100)}%`}
        htmlFor="gamepad-deadzone"
        hint="How far a stick must move before it counts. Raise it if the selection drifts on its own."
      >
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
      </FormField>
      <SaveError error={error} />

      <div className="pt-1">
        <Button onClick={toggleHelp}>Show the button map</Button>
      </div>
    </Section>
  );

  function commit() {
    if (dragged === null || dragged === stored) return;
    void save({ gamepadDeadzone: dragged }).then(() => setDragged(null));
  }
}
