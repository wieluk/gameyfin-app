import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";

import { SavePullPrompt } from "./SavePullPrompt";
import type { SavePullOffer } from "@/bindings/SavePullOffer";
import type { SaveVersion } from "@/types";

const listeners: Array<(payload: SavePullOffer) => void> = [];

vi.mock("@/lib/useTauriEvent", () => ({
  useTauriEvent: (_event: string, handler: (payload: SavePullOffer) => void) => {
    if (!listeners.includes(handler)) listeners.push(handler);
  },
}));

const answerSavePullOffer = vi.fn(async (_gameId: number, _saveId: string | null) => {});
const launch = vi.fn(async (_gameId: number) => {});

vi.mock("@/lib/backend", () => ({
  backend: {
    answerSavePullOffer: (gameId: number, saveId: string | null) =>
      answerSavePullOffer(gameId, saveId),
    launch: (gameId: number) => launch(gameId),
  },
}));

function version(id: string, platform: string): SaveVersion {
  return {
    id,
    gameId: 1,
    gameTitle: null,
    sizeBytes: 10,
    contentHash: id,
    platform,
    installationId: null,
    deviceName: `PC ${id}`,
    ludusaviTitle: null,
    locked: false,
    createdAt: null,
  };
}

function offer(localSaves: boolean): SavePullOffer {
  return {
    gameId: 1,
    title: "Duckov",
    versions: [
      { version: version("c", "MACOS"), restorable: false },
      { version: version("b", "WINDOWS"), restorable: true },
      { version: version("a", "WINDOWS"), restorable: true },
    ],
    localSaves,
    localAt: null,
  };
}

function show(next: SavePullOffer) {
  listeners.length = 0;
  answerSavePullOffer.mockClear();
  launch.mockClear();
  render(
    <QueryClientProvider client={new QueryClient()}>
      <SavePullPrompt />
    </QueryClientProvider>,
  );
  act(() => {
    for (const listener of [...listeners]) listener(next);
  });
}

describe("SavePullPrompt", () => {
  it("offers every stored save and restores the one picked", async () => {
    show(offer(true));
    const radios = screen.getAllByRole("radio");
    expect(radios).toHaveLength(3);
    // The newest one that restores here is the default; one that cannot is not pickable.
    expect((radios[0] as HTMLInputElement).disabled).toBe(true);
    expect((radios[1] as HTMLInputElement).checked).toBe(true);

    fireEvent.click(radios[2]);
    fireEvent.click(screen.getByRole("button", { name: "Restore and play" }));
    await waitFor(() => expect(answerSavePullOffer).toHaveBeenCalledWith(1, "a"));
    await waitFor(() => expect(launch).toHaveBeenCalledWith(1));
  });

  it("asks even when this PC has no save files, and can start without one", async () => {
    show(offer(false));
    fireEvent.click(screen.getByRole("button", { name: "Play without a save" }));
    await waitFor(() => expect(answerSavePullOffer).toHaveBeenCalledWith(1, null));
  });
});
