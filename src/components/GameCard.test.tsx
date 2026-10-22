import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { GameCard } from "./GameCard";
import { testEntry, testGame } from "@/lib/testing";

function card(onOpen = vi.fn()) {
  render(
    <GameCard
      entry={testEntry({ kind: "not-installed" }, { game: testGame({ title: "Celeste" }) })}
      coverSizes="320px"
      onPrimaryAction={() => {}}
      onOpen={onOpen}
    />,
  );
  // The cover tile, not the action button inside it.
  return screen.getAllByRole("button").find((el) => el.tagName === "DIV")!;
}

describe("GameCard", () => {
  it("lets go of focus after a mouse click, so closing the game leaves no ring", () => {
    const onOpen = vi.fn();
    const tile = card(onOpen);
    tile.focus();
    fireEvent.click(tile, { detail: 1 });
    expect(onOpen).toHaveBeenCalledOnce();
    expect(document.activeElement).not.toBe(tile);
  });

  it("keeps focus after a keyboard or controller open, so the user keeps their place", () => {
    const tile = card();
    tile.focus();
    // What Enter and the controller's `click()` produce.
    fireEvent.click(tile, { detail: 0 });
    expect(document.activeElement).toBe(tile);
  });
});
