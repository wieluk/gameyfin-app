import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";

import { SaveVersionList } from "./SaveVersionList";
import type { SaveVersion } from "@/types";

const versions: SaveVersion[] = ["b", "a"].map((id) => ({
  id,
  gameId: 1,
  gameTitle: null,
  sizeBytes: 10,
  contentHash: id,
  platform: "WINDOWS",
  installationId: null,
  deviceName: "Desk",
  ludusaviTitle: null,
  locked: false,
  createdAt: null,
}));

const deleteSaveVersions = vi.fn(async () => {});

vi.mock("@/lib/backend", () => ({
  backend: {
    listSaveVersions: async () => versions,
    deleteSaveVersions: (...args: unknown[]) => deleteSaveVersions(...(args as [])),
    setSaveLocked: async () => {},
  },
}));

function show() {
  render(
    <QueryClientProvider client={new QueryClient()}>
      <SaveVersionList gameId={1} busy={false} onRestore={() => {}} />
    </QueryClientProvider>,
  );
}

describe("SaveVersionList", () => {
  it("deletes a version only once the user confirms", async () => {
    show();
    const [first] = await screen.findAllByRole("button", { name: "Delete this version" });
    fireEvent.click(first);

    expect(screen.getByText("Delete for good?")).toBeTruthy();
    expect(deleteSaveVersions).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(deleteSaveVersions).toHaveBeenCalledWith(1, ["b"]));
  });

  it("asks before deleting every version at once", async () => {
    deleteSaveVersions.mockClear();
    show();
    fireEvent.click(await screen.findByRole("button", { name: "Delete all saves for this game" }));

    expect(screen.getByText(/Delete all 2 saves for this game\?/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Delete all" }));
    await waitFor(() => expect(deleteSaveVersions).toHaveBeenCalledWith(1, ["b", "a"]));
  });
});
