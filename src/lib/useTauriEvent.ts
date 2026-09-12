/**
 * Subscribe to a backend event while a component is mounted. Listening is async, so an
 * unmount before the listener is ready still has to undo it.
 */

import { useEffect, useRef } from "react";

import { isMockBackend } from "@/lib/backend";

export function useTauriEvent<T>(
  event: string,
  handler: (payload: T) => void,
  enabled = true,
): void {
  // Through a ref so a handler redefined on every render does not resubscribe.
  const latest = useRef(handler);
  latest.current = handler;

  useEffect(() => {
    if (isMockBackend || !enabled) return;
    let stop: (() => void) | undefined;
    let cancelled = false;

    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      const unlisten = await listen<T>(event, (e) => latest.current(e.payload));
      if (cancelled) unlisten();
      else stop = unlisten;
    })();

    return () => {
      cancelled = true;
      stop?.();
    };
  }, [event, enabled]);
}
