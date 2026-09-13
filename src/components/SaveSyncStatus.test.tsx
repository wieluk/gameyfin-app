import { act, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
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

function emit(phase: SaveSyncProgress["phase"], skippable: boolean, blocking = false) {
  const progress: SaveSyncProgress = {
    gameId: 1,
    title: "Celeste",
    moment: "launch",
    phase,
    skippable,
    blocking,
  };
  // Through act: the event arrives from outside React, as it does from Tauri.
  act(() => {
    for (const listener of [...listeners]) listener(progress);
  });
}

function show() {
  const client = new QueryClient();
  return render(
    <MemoryRouter>
      <QueryClientProvider client={client}>
        <SaveSyncStatus />
      </QueryClientProvider>
    </MemoryRouter>,
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

  it("holds a launch whose save could not be restored until the user chooses", () => {
    // Playing on silently is how a good save gets buried under an empty one.
    show();
    emit({ kind: "failed", message: "The save could not be put back." }, false, true);
    expect(screen.getByRole("button", { name: "Start anyway" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Open Saves" })).toBeTruthy();
    expect(screen.getByText(/The game has not started/)).toBeTruthy();
  });

  it("says a save was restored and where, not that it was already up to date", () => {
    show();
    emit(
      {
        kind: "restored",
        files: 2,
        folders: ["/home/u/Prefixes/1902/drive_c/users/u/AppData/LocalLow/TeamSoda/Duckov/Saves"],
        savedAt: null,
        device: "Desk",
      },
      false,
    );
    expect(screen.getByText(/Restored the save made .* on Desk: 2 files to .*Duckov\/Saves\./)).toBeTruthy();
    expect(screen.queryByText("Your save is already up to date.")).toBeNull();
  });

  it("says why nothing was restored instead of a catch-all", () => {
    show();
    emit({ kind: "done", state: { kind: "in-sync", lastSyncedAt: null } }, false);
    expect(screen.getByText("Your save is already up to date.")).toBeTruthy();

    emit({ kind: "done", state: { kind: "unmatched", candidates: [] } }, false);
    expect(screen.getByText(/not in the save location database/)).toBeTruthy();

    emit({ kind: "kept-local" }, false);
    expect(screen.getByText(/Keeping this PC's save/)).toBeTruthy();
  });
});
