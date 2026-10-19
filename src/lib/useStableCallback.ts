import { useCallback, useLayoutEffect, useRef } from "react";

/** A callback whose identity never changes but always runs the latest `fn`, so memoized children skip re-renders. */
export function useStableCallback<A extends unknown[], R>(fn: (...args: A) => R): (...args: A) => R {
  const latest = useRef(fn);
  useLayoutEffect(() => {
    latest.current = fn;
  });
  return useCallback((...args: A) => latest.current(...args), []);
}
