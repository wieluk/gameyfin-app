import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { Button, Switch, SwitchField, TextField } from "@/components/ui";

describe("Switch", () => {
  it("is a switch that reports the flipped value", () => {
    function Harness() {
      const [on, setOn] = useState(false);
      return <SwitchField label="Sync my saves" checked={on} onChange={setOn} />;
    }
    render(<Harness />);

    const control = screen.getByRole("switch", { name: "Sync my saves" });
    expect(control.getAttribute("aria-checked")).toBe("false");
    fireEvent.click(control);
    expect(control.getAttribute("aria-checked")).toBe("true");
  });

  it("toggles from its label", () => {
    const onChange = vi.fn();
    render(<Switch label="Show all games" checked={false} onChange={onChange} />);

    fireEvent.click(screen.getByText("Show all games"));
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it("groups the settings that belong to it under its name", () => {
    render(
      <SwitchField label="Install automatically" checked onChange={() => {}}>
        <SwitchField label="Delete the download" checked={false} onChange={() => {}} />
      </SwitchField>,
    );

    const group = screen.getByRole("group", { name: "Install automatically" });
    expect(group.contains(screen.getByRole("switch", { name: "Delete the download" }))).toBe(true);
  });

  it("ignores presses while disabled", () => {
    const onChange = vi.fn();
    render(<Switch aria-label="Off limits" checked disabled onChange={onChange} />);

    fireEvent.click(screen.getByRole("switch", { name: "Off limits" }));
    expect(onChange).not.toHaveBeenCalled();
  });
});

describe("TextField", () => {
  it("saves on leaving the box, not per keystroke", () => {
    const onCommit = vi.fn();
    render(<TextField label="Device name" value="Desk" onCommit={onCommit} />);

    const box = screen.getByLabelText("Device name");
    fireEvent.change(box, { target: { value: "Couch" } });
    expect(onCommit).not.toHaveBeenCalled();

    fireEvent.blur(box);
    expect(onCommit).toHaveBeenCalledWith("Couch");
  });

  it("does not save a value that did not change", () => {
    const onCommit = vi.fn();
    render(<TextField label="Device name" value="Desk" onCommit={onCommit} />);

    fireEvent.blur(screen.getByLabelText("Device name"));
    expect(onCommit).not.toHaveBeenCalled();
  });
});

describe("Button", () => {
  it("announces a toggle's state only when it is one", () => {
    render(
      <>
        <Button pressed>Desktop</Button>
        <Button>Open folder</Button>
      </>,
    );

    expect(screen.getByRole("button", { name: "Desktop" }).getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByRole("button", { name: "Open folder" }).hasAttribute("aria-pressed")).toBe(false);
  });
});
