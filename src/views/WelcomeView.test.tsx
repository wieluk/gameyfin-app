import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { WelcomeView } from "./WelcomeView";

vi.mock("@/lib/useTauriEvent", () => ({ useTauriEvent: () => undefined }));

const settings = { libraryRoot: null as string | null, deviceName: null as string | null };
const updateSettings = vi.fn(async (_patch: unknown) => {});

vi.mock("@/lib/backend", () => ({
  backend: {
    beginLogin: async () => {},
    cancelLogin: async () => {},
    pollLogin: async () => ({ signedIn: true, windowOpen: false, detail: null }),
    getSettings: async () => settings,
    suggestLibraryRoot: async () => "/home/me/Games",
    detectedDeviceName: async () => "desk",
    updateSettings: (patch: unknown) => updateSettings(patch),
  },
}));

// Sign in is polled once a second.
const POLLED = { timeout: 3000 };

function show(knownServer: string | null) {
  const onStarted = vi.fn();
  const onComplete = vi.fn();
  render(
    <QueryClientProvider client={new QueryClient()}>
      <WelcomeView knownServer={knownServer} onStarted={onStarted} onComplete={onComplete} />
    </QueryClientProvider>,
  );
  return { onStarted, onComplete };
}

describe("WelcomeView", () => {
  beforeEach(() => {
    settings.libraryRoot = null;
    settings.deviceName = null;
    updateSettings.mockClear();
  });

  it("starts at the server step on a first run", () => {
    show(null);
    expect(screen.getByText("Welcome to Gameyfin")).toBeTruthy();
    expect(screen.getByLabelText("Server address")).toBeTruthy();
  });

  it("resumes at sign in and finishes when a folder is already chosen", async () => {
    settings.libraryRoot = "/data/games";
    const { onStarted, onComplete } = show("https://games.example");

    expect(screen.getByText("Welcome back")).toBeTruthy();
    expect(screen.getByText("https://games.example")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
    await waitFor(() => expect(onComplete).toHaveBeenCalled(), POLLED);
    expect(onStarted).toHaveBeenCalled();
    expect(updateSettings).not.toHaveBeenCalled();
  });

  it("asks for a folder after sign in when none was chosen, keeping the saved device name", async () => {
    settings.deviceName = "Living room";
    const { onComplete } = show("https://games.example");

    fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
    const folder = await screen.findByLabelText<HTMLInputElement>("Games folder", {}, POLLED);
    await waitFor(() => expect(folder.value).toBe("/home/me/Games"));
    expect(screen.getByDisplayValue("Living room")).toBeTruthy();
    expect(onComplete).not.toHaveBeenCalled();
  });
});
