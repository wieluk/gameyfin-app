import { act, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";

import { SaveSyncStatus } from "./SaveSyncStatus";
import type { SaveSyncProgress } from "@/bindings/SaveSyncProgress";

/** The event the backend emits, which outside Tauri has to be delivered by hand. */
const listeners: Array<(payload: SaveSyncProgress) => void> = [];

vi.mock("@/lib/useTauriEvent", () => ({
  useTauriEvent: (_event: string, handler: (payload: SaveSyncProgress) => void) => {
    if (!listeners.includes(handler)) listeners.push(handler);
  },
}));

function emit(phase: SaveSyncProgress["phase"], skippable: boolean) {
  const progress: SaveSyncProgress = {
    gameId: 1,
    title: "Celeste",
    moment: "launch",
    phase,
    skippable,
  };
  // Through act: the event arrives from outside React, as it does from Tauri.
  act(() => {
    for (const listener of [...listeners]) listener(progress);
  });
}

function show() {
  const client = new QueryClient();
  return render(
    <QueryClientProvider client={client}>
      <SaveSyncStatus />
    </QueryClientProvider>,
  );
}

describe("SaveSyncStatus", () => {
  it("says nothing until a sync starts", () => {
    show();
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("offers Skip while it is safe and refuses once it is not", () => {
    show();
    emit({ kind: "checking" }, true);
    expect(screen.getByRole("button", { name: "Skip" }).hasAttribute("disabled")).toBe(false);

    // Files are being written now, so stopping would leave half a save.
    emit({ kind: "restoring" }, false);
    expect(screen.getByRole("button", { name: "Skip" }).hasAttribute("disabled")).toBe(true);
  });

  it("keeps a failure on screen with its message", () => {
    show();
    emit({ kind: "failed", message: "the server refused the save" }, false);
    expect(screen.getByText("the server refused the save")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Close" })).toBeTruthy();
  });
});
