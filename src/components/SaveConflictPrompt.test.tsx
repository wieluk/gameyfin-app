import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";

import { SaveConflictPrompt } from "./SaveConflictPrompt";
import type { SaveConflictPrompt as Prompt } from "@/bindings/SaveConflictPrompt";

const listeners: Array<(payload: Prompt) => void> = [];

vi.mock("@/lib/useTauriEvent", () => ({
  useTauriEvent: (_event: string, handler: (payload: Prompt) => void) => {
    if (!listeners.includes(handler)) listeners.push(handler);
  },
}));

const resolveSaveConflict = vi.fn(async (_gameId: number, _choice: string) => ({
  kind: "in-sync",
  lastSyncedAt: null,
}));
const launch = vi.fn(async (_gameId: number) => {});

vi.mock("@/lib/backend", () => ({
  backend: {
    resolveSaveConflict: (gameId: number, choice: string) => resolveSaveConflict(gameId, choice),
    launch: (gameId: number) => launch(gameId),
  },
}));

function prompt(moment: "launch" | "exit"): Prompt {
  return {
    gameId: 1,
    title: "Duckov",
    moment,
    localAt: null,
    remote: {
      id: "9",
      gameId: 1,
      gameTitle: null,
      sizeBytes: 10,
      contentHash: "h",
      platform: "WINDOWS",
      installationId: null,
      deviceName: "Desk",
      ludusaviTitle: null,
      locked: false,
      createdAt: null,
    },
  };
}

function show(next: Prompt) {
  listeners.length = 0;
  resolveSaveConflict.mockClear();
  launch.mockClear();
  render(
    <QueryClientProvider client={new QueryClient()}>
      <SaveConflictPrompt />
    </QueryClientProvider>,
  );
  act(() => listeners.forEach((listener) => listener(next)));
}

describe("SaveConflictPrompt", () => {
  it("starts the game a launch was holding once the conflict is settled", async () => {
    show(prompt("launch"));
    fireEvent.click(screen.getByRole("button", { name: "Keep the cloud save" }));
    await waitFor(() => expect(launch).toHaveBeenCalledWith(1));
    expect(resolveSaveConflict).toHaveBeenCalledWith(1, "keep-remote");
  });

  it("leaves the game unstarted on Not now", () => {
    show(prompt("launch"));
    fireEvent.click(screen.getByRole("button", { name: "Not now" }));
    expect(resolveSaveConflict).not.toHaveBeenCalled();
    expect(launch).not.toHaveBeenCalled();
  });

  it("starts nothing after playing", async () => {
    show(prompt("exit"));
    fireEvent.click(screen.getByRole("button", { name: "Keep this PC's save" }));
    await waitFor(() => expect(resolveSaveConflict).toHaveBeenCalledWith(1, "keep-local"));
    expect(launch).not.toHaveBeenCalled();
  });
});
