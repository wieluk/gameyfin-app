import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { SetupProgram } from "@/bindings/SetupProgram";
import { SetupList, SetupPicker } from "./SetupList";

function setup(path: string, overrides: Partial<SetupProgram> = {}): SetupProgram {
  return {
    path,
    role: "patch",
    superseded: false,
    installed: false,
    silent: true,
    recommended: true,
    matched: true,
    ...overrides,
  };
}

const three = [setup("setup.exe", { role: "game" }), setup("patch1.exe"), setup("patch2.exe")];

describe("SetupList", () => {
  it("nudges a row up or down, and not past either end", () => {
    const onMove = vi.fn();
    render(<SetupList setups={three} selected={[]} onToggle={() => {}} onMove={onMove} />);

    fireEvent.click(screen.getByRole("button", { name: "Move patch1.exe down" }));
    expect(onMove).toHaveBeenCalledWith(1, 2);
    fireEvent.click(screen.getByRole("button", { name: "Move patch1.exe up" }));
    expect(onMove).toHaveBeenCalledWith(1, 0);
    expect(
      screen.getByRole("button", { name: "Move setup.exe up" }).hasAttribute("disabled"),
    ).toBe(true);
  });

  it("offers no reordering where the order is not the user's to change", () => {
    render(<SetupList setups={three} selected={[]} onToggle={() => {}} />);
    expect(screen.queryByRole("button", { name: /Move/ })).toBeNull();
  });
});

describe("SetupPicker", () => {
  it("moves a ticked program into the ordered list and reports the whole order", () => {
    const onOrder = vi.fn();
    const withOther = [...three, setup("tools/fix.exe", { role: "other", matched: false })];
    render(
      <SetupPicker
        setups={withOther}
        selected={["tools/fix.exe"]}
        onToggle={() => {}}
        onOrder={onOrder}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Move tools/fix.exe up" }));
    expect(onOrder).toHaveBeenCalledWith(["setup.exe", "patch1.exe", "tools/fix.exe", "patch2.exe"]);
  });
});
