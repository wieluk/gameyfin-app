import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { useSetupGate } from "./useSetupGate";

function Screen({ needsSetup }: { needsSetup: boolean }) {
  const gate = useSetupGate(needsSetup);
  return (
    <>
      <p>{gate.open ? "wizard" : "app"}</p>
      <button type="button" onClick={gate.engage}>
        engage
      </button>
      <button type="button" onClick={gate.close}>
        finish
      </button>
    </>
  );
}

function showing() {
  return screen.getByRole("paragraph").textContent;
}

function press(name: string) {
  fireEvent.click(screen.getByRole("button", { name }));
}

describe("useSetupGate", () => {
  it("keeps the wizard up when signing in mid-flow answers the status query", () => {
    const view = render(<Screen needsSetup={true} />);
    expect(showing()).toBe("wizard");
    press("engage");

    // Step two signed in, so setup is no longer "needed"; the user has not finished it.
    view.rerender(<Screen needsSetup={false} />);
    expect(showing()).toBe("wizard");

    press("finish");
    expect(showing()).toBe("app");
  });

  it("stays closed while the status query catches up with the session just created", () => {
    // Finishing before the refetch lands: the cached status still says "not signed in",
    // which without the dismissed state would put the wizard straight back on screen.
    const view = render(<Screen needsSetup={true} />);
    press("engage");
    press("finish");
    expect(showing()).toBe("app");

    view.rerender(<Screen needsSetup={true} />);
    expect(showing()).toBe("app");
  });

  it("goes away again when a session comes back before the user answers anything", () => {
    const view = render(<Screen needsSetup={true} />);
    expect(showing()).toBe("wizard");
    view.rerender(<Screen needsSetup={false} />);
    expect(showing()).toBe("app");
  });

  it("stays out of the way for a configured install", () => {
    render(<Screen needsSetup={false} />);
    expect(showing()).toBe("app");
  });

  it("comes back when the session is gone, such as after signing out", () => {
    const view = render(<Screen needsSetup={false} />);
    view.rerender(<Screen needsSetup={true} />);
    expect(showing()).toBe("wizard");
  });

  it("comes back after a finished setup when the session is lost later", () => {
    const view = render(<Screen needsSetup={true} />);
    press("engage");
    press("finish");
    // The session the wizard created shows up, and is lost again some time later.
    view.rerender(<Screen needsSetup={false} />);
    view.rerender(<Screen needsSetup={true} />);
    expect(showing()).toBe("wizard");
  });
});
