import { act, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SaveSyncStatus } from "./SaveSyncStatus";
import { SHOW_AFTER_MS } from "@/lib/syncWindow";
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

/** Lets a step run long enough to be worth a window. */
function wait() {
  act(() => {
    vi.advanceTimersByTime(SHOW_AFTER_MS);
  });
}

function show() {
  listeners.length = 0;
  const client = new QueryClient();
  return render(
    <MemoryRouter>
      <QueryClientProvider client={client}>
        <SaveSyncStatus />
      </QueryClientProvider>
    </MemoryRouter>,
  );
}

afterEach(() => {
  vi.useRealTimers();
});

describe("SaveSyncStatus", () => {
  it("says nothing until a sync starts", () => {
    show();
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("stays shut for a check that is over at once", () => {
    vi.useFakeTimers();
    show();
    emit({ kind: "checking" }, true);
    emit({ kind: "done", state: { kind: "in-sync", lastSyncedAt: null } }, false);
    wait();
    expect(screen.queryByRole("alertdialog")).toBeNull();
  });

  it("offers Skip while it is safe and refuses once it is not", () => {
    vi.useFakeTimers();
    show();
    emit({ kind: "checking" }, true);
    expect(screen.queryByRole("alertdialog")).toBeNull();
    wait();
    expect(screen.getByRole("button", { name: "Skip" }).hasAttribute("disabled")).toBe(false);

    // Files are being written now, so stopping would leave half a save.
    emit({ kind: "restoring" }, false);
    expect(screen.getByRole("button", { name: "Skip" }).hasAttribute("disabled")).toBe(true);
  });

  it("gives way to the save prompt without a message in between", () => {
    vi.useFakeTimers();
    show();
    emit({ kind: "checking" }, true);
    wait();
    emit({ kind: "asking" }, false);
    expect(screen.queryByRole("alertdialog")).toBeNull();
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

  it("says a save was restored and where, and keeps saying it over a quiet check", () => {
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
    const restored = /Restored the save made .* on Desk: 2 files to .*Duckov\/Saves\./;
    expect(screen.getByText(restored)).toBeTruthy();

    emit({ kind: "done", state: { kind: "in-sync", lastSyncedAt: null } }, false);
    expect(screen.getByText(restored)).toBeTruthy();
    expect(screen.queryByText("Your save is already up to date.")).toBeNull();
  });

  it("says why nothing was restored once the window is open", () => {
    vi.useFakeTimers();
    show();
    emit({ kind: "checking" }, true);
    wait();
    emit({ kind: "done", state: { kind: "in-sync", lastSyncedAt: null } }, false);
    expect(screen.getByText("Your save is already up to date.")).toBeTruthy();

    emit({ kind: "done", state: { kind: "unmatched", candidates: [] } }, false);
    expect(screen.getByText(/not in the save location database/)).toBeTruthy();

    emit({ kind: "kept-local" }, false);
    expect(screen.getByText(/Keeping this PC's save/)).toBeTruthy();
  });
});
