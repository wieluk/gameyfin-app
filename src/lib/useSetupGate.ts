import { useCallback, useEffect, useMemo, useState } from "react";

/**
 * When the first-run wizard is on screen. Once started it stays until Finish: signing in
 * part-way makes the status healthy, which would otherwise skip the remaining steps.
 */
type Mode = "idle" | "open" | "dismissed";

export function useSetupGate(needsSetup: boolean) {
  const [mode, setMode] = useState<Mode>(needsSetup ? "open" : "idle");
  // Until the user has answered a step, a session that comes back on its own (a server
  // that was restarting, say) should take the wizard away again rather than strand them in it.
  const [engaged, setEngaged] = useState(false);

  useEffect(() => {
    setMode((current) => {
      if (needsSetup) return current === "dismissed" ? current : "open";
      if (current === "open" && engaged) return current;
      return "idle";
    });
  }, [needsSetup, engaged]);

  const engage = useCallback(() => setEngaged(true), []);
  const close = useCallback(() => {
    setEngaged(false);
    // Not "idle": the status query still reports the session this wizard just created as
    // missing, and that would reopen the wizard the moment it closed.
    setMode("dismissed");
  }, []);

  return useMemo(
    () => ({ open: mode === "open", engage, close }),
    [mode, engage, close],
  );
}
