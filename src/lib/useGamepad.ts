/**
 * Binding controller events to the interface.
 *
 * One hook, mounted once by the shell. It owns no state of its own beyond the listeners:
 * what a button does is decided from what is on screen at the time, which is what lets the
 * same bindings work in the library, in a dialog and in Settings without any of them
 * knowing a controller exists.
 */

import { useEffect, useRef } from "react";
import { useLocation, useNavigate } from "react-router-dom";
import { useQueryClient } from "@tanstack/react-query";

import { isMockBackend } from "@/lib/backend";
import {
  activateFocused,
  clearPadFocus,
  goBack,
  moveFocus,
  nextTab,
  scrollByStick,
  scrollPage,
  type AxisEvent,
  type ButtonEvent,
  type ConnectionEvent,
} from "@/lib/gamepad";
import { useAppSettings } from "@/lib/queries";
import { useCouch } from "@/state/couch";

export function useGamepad() {
  const navigate = useNavigate();
  const location = useLocation();
  const queryClient = useQueryClient();
  const settings = useAppSettings();
  const { setConnected, setCouch, toggleHelp, closeHelp } = useCouch();

  const couchAuto = settings.data?.couchModeAuto ?? true;

  // Read through a ref so listeners stay mounted; re-subscribing on nav would drop presses.
  const context = useRef({ pathname: location.pathname, couchAuto });
  context.current = { pathname: location.pathname, couchAuto };

  // The controller ring is drawn from an explicit flag, so something has to take it back
  // when the user reaches for the mouse.
  useEffect(() => {
    document.addEventListener("pointerdown", clearPadFocus);
    return () => document.removeEventListener("pointerdown", clearPadFocus);
  }, []);

  useEffect(() => {
    if (isMockBackend) return;
    let cancelled = false;
    const unlisteners: Array<() => void> = [];

    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");

      const button = await listen<ButtonEvent>("gamepad-button", (event) => {
        handleButton(event.payload);
      });
      const axis = await listen<AxisEvent>("gamepad-axis", (event) => {
        scrollByStick(event.payload);
      });
      const connection = await listen<ConnectionEvent>("gamepad-connection", (event) => {
        const { connected, name } = event.payload;
        setConnected(connected, name);
        // The large layout follows the pad by default.
        if (connected && context.current.couchAuto) setCouch(true);
      });

      if (cancelled) {
        button();
        axis();
        connection();
        return;
      }
      unlisteners.push(button, axis, connection);
    })();

    function handleButton({ button, repeat }: ButtonEvent) {
      switch (button) {
        case "up":
        case "down":
        case "left":
        case "right":
          moveFocus(button);
          return;

        case "south":
          // A held A must not repeat: one press should not start a download or game twice.
          if (!repeat) activateFocused();
          return;

        case "east":
          if (!repeat) {
            // The overlay is the topmost thing when it is open, so B closes that first.
            if (useCouch.getState().helpOpen) closeHelp();
            else goBack();
          }
          return;

        case "north":
          if (!repeat) void queryClient.invalidateQueries();
          return;

        case "left-bumper":
          navigate(nextTab(context.current.pathname, -1));
          return;
        case "right-bumper":
          navigate(nextTab(context.current.pathname, 1));
          return;

        case "left-trigger":
          scrollPage(-1);
          return;
        case "right-trigger":
          scrollPage(1);
          return;

        case "start":
          if (!repeat) toggleHelp();
          return;

        // Select is deliberately unbound: nothing here it obviously means.
        case "select":
        case "west":
          return;
      }
    }

    return () => {
      cancelled = true;
      unlisteners.forEach((off) => off());
    };
    // Mounted once; changing values are read through `context` or the store.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
}
