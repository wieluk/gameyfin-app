import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { FixMatch } from "@/bindings/FixMatch";
import { describeFix, FixPickerDialog } from "./FixPickerDialog";

const found: FixMatch[] = [
  { title: "The Witcher 2: Assassins of Kings", fix: { id: "umu-20920", store: "gog" } },
];

vi.mock("@/lib/backend", () => ({
  backend: { searchGameFixes: vi.fn(async () => found) },
}));

describe("describeFix", () => {
  it("names the store only where it decides which fix runs", () => {
    expect(describeFix({ id: "umu-default", store: "none" })).toBe("no fix");
    expect(describeFix({ id: "umu-20920", store: "gog" })).toBe("umu-20920 (GOG)");
    expect(describeFix({ id: "umu-starcitizen", store: "none" })).toBe("umu-starcitizen");
    // A bare number is looked up as a Steam AppID whatever the store says.
    expect(describeFix({ id: "292030", store: "gog" })).toBe("Steam fix 292030");
  });
});

describe("FixPickerDialog", () => {
  function open(onChoose = vi.fn(async () => {}), onClose = vi.fn()) {
    render(
      <FixPickerDialog
        gameTitle="The Witcher 3: Wild Hunt"
        steamFix={{ id: "292030", store: "none" }}
        onChoose={onChoose}
        onClose={onClose}
      />,
    );
    return { onChoose, onClose };
  }

  it("saves a row from the list and closes", async () => {
    const { onChoose, onClose } = open();
    fireEvent.click(await screen.findByText("The Witcher 2: Assassins of Kings"));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(onChoose).toHaveBeenCalledWith({ id: "umu-20920", store: "gog" });
  });

  it("offers the fix for the Steam release ahead of the list", async () => {
    const { onChoose } = open();
    fireEvent.click(screen.getByText("The fix for this game’s Steam release"));
    await waitFor(() => expect(onChoose).toHaveBeenCalledWith({ id: "292030", store: "none" }));
  });

  it("takes a typed id with its store, and shows why one was refused", async () => {
    const onChoose = vi.fn(async () => {
      throw new Error("not a fix id");
    });
    const { onClose } = open(onChoose);
    fireEvent.change(screen.getByLabelText("Fix id"), { target: { value: "umu-1234" } });
    fireEvent.change(screen.getByLabelText("Store"), { target: { value: "egs" } });
    fireEvent.click(screen.getByRole("button", { name: "Use" }));
    expect(await screen.findByText("not a fix id")).toBeTruthy();
    expect(onChoose).toHaveBeenCalledWith({ id: "umu-1234", store: "egs" });
    expect(onClose).not.toHaveBeenCalled();
  });
});
