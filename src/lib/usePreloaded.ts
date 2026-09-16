import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Opens a dialog only once what it shows has loaded, so it never appears half-empty and then
 * rearranges itself. `loading` names the target meanwhile, for the button that asked. A failed
 * load rejects `open`, for the caller to report the way it reports anything else.
 */
export function usePreloaded<T, D>(load: (target: T) => Promise<D>) {
  const [opened, setOpened] = useState<{ target: T; data: D } | null>(null);
  const [loading, setLoading] = useState<T | null>(null);
  // Only the latest request may open anything, or a slow earlier one would open over it.
  const latest = useRef(0);
  const loader = useRef(load);
  useEffect(() => {
    loader.current = load;
  });
  useEffect(
    () => () => {
      latest.current++;
    },
    [],
  );

  const open = useCallback(async (target: T) => {
    const request = ++latest.current;
    setLoading(target);
    try {
      const data = await loader.current(target);
      if (latest.current === request) setOpened({ target, data });
    } catch (e) {
      if (latest.current === request) throw e;
    } finally {
      if (latest.current === request) setLoading(null);
    }
  }, []);

  const close = useCallback(() => {
    latest.current++;
    setOpened(null);
    setLoading(null);
  }, []);

  return { opened, loading, open, close };
}
