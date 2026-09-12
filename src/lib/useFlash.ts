/** A flag that turns itself off, for the "Saved" tick that must not outlive the component. */

import { useCallback, useEffect, useRef, useState } from "react";

export function useFlash(milliseconds = 2000): [boolean, () => void] {
  const [shown, setShown] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  useEffect(() => () => clearTimeout(timer.current), []);

  const flash = useCallback(() => {
    setShown(true);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setShown(false), milliseconds);
  }, [milliseconds]);

  return [shown, flash];
}
