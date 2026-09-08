import { render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { useDismissOnEscape } from "./useDismiss";

function Dialog({ onDismiss }: { onDismiss: () => void }) {
  useDismissOnEscape(onDismiss);
  return null;
}

function pressEscape() {
  document.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }),
  );
}

describe("useDismissOnEscape", () => {
  it("closes the dialog that is open", () => {
    const onDismiss = vi.fn();
    render(<Dialog onDismiss={onDismiss} />);
    pressEscape();
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("closes only the topmost of two", () => {
    // One listener per dialog would close both on a single press, which is the bug this
    // stack exists to avoid.
    const outer = vi.fn();
    const inner = vi.fn();
    render(<Dialog onDismiss={outer} />);
    const nested = render(<Dialog onDismiss={inner} />);

    pressEscape();
    expect(inner).toHaveBeenCalledTimes(1);
    expect(outer).not.toHaveBeenCalled();

    // With the inner one gone, the press reaches the one behind it.
    nested.unmount();
    pressEscape();
    expect(outer).toHaveBeenCalledTimes(1);
  });

  it("stops listening once every dialog has closed", () => {
    const onDismiss = vi.fn();
    const dialog = render(<Dialog onDismiss={onDismiss} />);
    dialog.unmount();
    pressEscape();
    expect(onDismiss).not.toHaveBeenCalled();
  });

  it("calls the handler as it is at press time", () => {
    // The handler is re-created on every render of a real dialog; the stack must not
    // pin the one from mount.
    const first = vi.fn();
    const second = vi.fn();
    const dialog = render(<Dialog onDismiss={first} />);
    dialog.rerender(<Dialog onDismiss={second} />);

    pressEscape();
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledTimes(1);
  });
});
