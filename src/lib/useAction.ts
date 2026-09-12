/**
 * Run a backend call with the busy flag and error line every control needs. Without it a
 * dropped promise is a button that silently does nothing.
 */

import { useCallback, useEffect, useRef, useState } from "react";

import { messageOf } from "@/lib/errors";

export function useAction() {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const run = useCallback(async <T,>(work: () => Promise<T>): Promise<T | undefined> => {
    setBusy(true);
    setError(null);
    try {
      return await work();
    } catch (e) {
      if (mounted.current) setError(messageOf(e));
      return undefined;
    } finally {
      if (mounted.current) setBusy(false);
    }
  }, []);

  return { run, busy, error, setError };
}
