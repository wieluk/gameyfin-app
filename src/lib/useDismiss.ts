/**
 * Escape closes the topmost dialog. A stack, so a nested dialog and the one behind it do not
 * both close on one press. Controller Back arrives here as a dispatched Escape.
 */

import { useEffect, useRef } from "react";

const stack: Array<() => void> = [];

function onKeyDown(event: KeyboardEvent) {
  if (event.key !== "Escape") return;
  const top = stack[stack.length - 1];
  if (!top) return;
  event.preventDefault();
  top();
}

/** Call `onDismiss` when Escape is pressed and this is the topmost dialog. */
export function useDismissOnEscape(onDismiss: () => void) {
  // Through a ref so a handler redefined on every render does not re-order the stack.
  const latest = useRef(onDismiss);
  latest.current = onDismiss;

  useEffect(() => {
    const entry = () => latest.current();
    stack.push(entry);
    if (stack.length === 1) document.addEventListener("keydown", onKeyDown);

    return () => {
      const at = stack.indexOf(entry);
      if (at !== -1) stack.splice(at, 1);
      if (stack.length === 0) document.removeEventListener("keydown", onKeyDown);
    };
  }, []);
}
