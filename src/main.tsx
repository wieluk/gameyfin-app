import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { BrowserRouter } from "react-router-dom";
import { App } from "./App";
import { ErrorBoundary } from "@/components/ErrorBoundary";
import { backend } from "@/lib/backend";
import "./styles.css";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // The Rust core pushes changes over events, so background refetching is noise.
      refetchOnWindowFocus: false,
      staleTime: 30_000,
    },
  },
});

// Set by the backend when it rebuilds a window it let go in the tray, opened from a menu entry.
const startRoute = (window as { __GAMEYFIN_START_ROUTE__?: string }).__GAMEYFIN_START_ROUTE__;
if (startRoute) window.history.replaceState(null, "", startRoute);

// A few seconds of frames show the renderer survives on this machine; a hidden window waits until shown.
requestAnimationFrame(() =>
  setTimeout(() => requestAnimationFrame(() => void backend.confirmRendering().catch(() => {})), 3000),
);

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ErrorBoundary>
      <QueryClientProvider client={queryClient}>
        <BrowserRouter>
          <App />
        </BrowserRouter>
      </QueryClientProvider>
    </ErrorBoundary>
  </React.StrictMode>,
);
