/**
 * Escape closes the topmost dialog.
 *
 * A stack rather than a listener per dialog: with one listener each, a nested dialog and
 * the one behind it would both close on a single press. Controller Back arrives here too,
 * as a dispatched Escape, so binding it once covers both.
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
