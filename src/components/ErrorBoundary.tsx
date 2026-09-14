import React from "react";
import { Button } from "@/components/ui";
import { backend, isMockBackend } from "@/lib/backend";

/**
 * Catches a render error so it does not leave a blank window: a packaged app has no console
 * and no obvious way to reload.
 */
export class ErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { error: Error | null }
> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("interface crashed", error, info.componentStack);
    // Also to the log file, which is the only record a packaged build leaves behind.
    if (!isMockBackend) {
      void backend
        .reportCrash(`${error.message}\n${info.componentStack ?? ""}`)
        .catch(() => undefined);
    }
  }

  render() {
    if (!this.state.error) return this.props.children;

    return (
      <div className="flex h-screen flex-col items-center justify-center gap-4 p-8 text-center">
        <h1 className="text-lg font-semibold">Gameyfin ran into a problem</h1>
        <p className="max-w-md text-sm text-foreground/60">
          The window could not finish drawing. Reloading usually clears it, and your
          downloads and saves are not affected.
        </p>
        <pre className="max-h-40 max-w-xl overflow-auto rounded-lg bg-content2 p-3 text-left text-[11px] text-foreground/70">
          {this.state.error.message}
        </pre>
        <Button variant="primary" size="lg" onClick={() => window.location.reload()}>
          Reload
        </Button>
      </div>
    );
  }
}
