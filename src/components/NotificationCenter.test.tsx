import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { describe, expect, it, vi } from "vitest";

import { NotificationBell } from "./NotificationCenter";
import type { AppNotification } from "@/bindings/AppNotification";

vi.mock("@/lib/useTauriEvent", () => ({ useTauriEvent: () => {} }));

const items: AppNotification[] = [
  {
    id: 2,
    category: "failure",
    title: "Install failed",
    body: "Celeste: exit code 1",
    createdAt: null as unknown as string,
    route: "/downloads",
    read: false,
  },
  {
    id: 1,
    category: "transfer",
    title: "Ready to play",
    body: "Hades is installed.",
    createdAt: null as unknown as string,
    route: "/installed",
    read: true,
  },
];

const dismissNotification = vi.fn(async (_id: number | null) => {});
const markNotificationsRead = vi.fn(async () => {});

vi.mock("@/lib/backend", () => ({
  backend: {
    listNotifications: async () => items,
    dismissNotification: (id: number | null) => dismissNotification(id),
    markNotificationsRead: () => markNotificationsRead(),
  },
}));

function show() {
  const client = new QueryClient();
  return render(
    <MemoryRouter initialEntries={["/"]}>
      <QueryClientProvider client={client}>
        <Routes>
          <Route path="/" element={<NotificationBell />} />
          <Route path="/downloads" element={<p>Downloads page</p>} />
        </Routes>
      </QueryClientProvider>
    </MemoryRouter>,
  );
}

describe("NotificationBell", () => {
  it("counts what has not been read, and opening the list reads it", async () => {
    show();
    const bell = await screen.findByRole("button", { name: "Notifications, 1 unread" });
    fireEvent.click(bell);
    expect(screen.getByText("Install failed")).toBeTruthy();
    expect(markNotificationsRead).toHaveBeenCalled();
  });

  it("goes to the page a notification is about and takes it off the list", async () => {
    show();
    fireEvent.click(await screen.findByRole("button", { name: /Notifications/ }));
    fireEvent.click(screen.getByText("Install failed"));
    await waitFor(() => expect(screen.getByText("Downloads page")).toBeTruthy());
    expect(dismissNotification).toHaveBeenCalledWith(2);
  });

  it("dismisses everything at once", async () => {
    show();
    fireEvent.click(await screen.findByRole("button", { name: /Notifications/ }));
    fireEvent.click(screen.getByRole("button", { name: "Dismiss all" }));
    expect(dismissNotification).toHaveBeenCalledWith(null);
  });
});
