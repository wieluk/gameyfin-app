import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { BrowserRouter } from "react-router-dom";
import { App } from "./App";
import { ErrorBoundary } from "@/components/ErrorBoundary";
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
