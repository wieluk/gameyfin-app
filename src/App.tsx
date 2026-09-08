import { useCallback, useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Route, Routes, useNavigate } from "react-router-dom";
import { Icon } from "@/components/Icon";
import { ResizeHandles } from "@/components/ResizeHandles";
import { Sidebar } from "@/components/Sidebar";
import { TitleBar } from "@/components/TitleBar";
import { isMockBackend } from "@/lib/backend";
import type { LibraryEntry } from "@/types";
import { DownloadsView } from "@/views/DownloadsView";
import { InstalledView } from "@/views/InstalledView";
import { LibraryView } from "@/views/LibraryView";
import { SettingsView } from "@/views/SettingsView";
import { WelcomeView } from "@/views/WelcomeView";
import { useEntries, useStatus } from "@/lib/queries";

export function App() {
  const queryClient = useQueryClient();
  const navigate = useNavigate();

  const status = useStatus();

  // The backend restores a stored session at startup; until that resolves, showing the
  // wizard would flash it in front of a user who is in fact already signed in.
  const [restoreSettled, setRestoreSettled] = useState(isMockBackend);
  useEffect(() => {
    if (isMockBackend) return;
    let cancelled = false;

    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      const unlisten = await listen("connection-restored", () => {
        if (cancelled) return;
        setRestoreSettled(true);
        void queryClient.invalidateQueries({ queryKey: ["status"] });
      });
      // If the event already fired before this listener attached, the status query
      // still settles things; don't wait forever on an event that will not come again.
      const timer = setTimeout(() => !cancelled && setRestoreSettled(true), 3000);
      return () => {
        clearTimeout(timer);
        unlisten();
      };
    })();

    return () => {
      cancelled = true;
    };
  }, [queryClient]);

  const onConnected = useCallback(async () => {
    await queryClient.invalidateQueries();
    navigate("/");
  }, [queryClient, navigate]);

  const ready = restoreSettled && !status.isLoading;
  const needsSetup = ready && !(status.data?.configured && status.data?.authenticated);
  const offline = Boolean(status.data?.offline);

  // Everything fetched while the server was down came from the local cache, so the moment
  // it answers again the whole lot is worth re-reading.
  const wasOffline = useRef(offline);
  useEffect(() => {
    if (wasOffline.current && !offline) void queryClient.invalidateQueries();
    wasOffline.current = offline;
  }, [offline, queryClient]);

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-background text-foreground">
      <ResizeHandles />
      <TitleBar />
      {isMockBackend && <MockBanner />}
      {offline && <OfflineBanner serverUrl={status.data?.serverUrl ?? null} />}

      {!ready ? (
        <Splash />
      ) : needsSetup ? (
        <WelcomeView onComplete={onConnected} />
      ) : (
        <Shell onSignedOut={() => void queryClient.invalidateQueries({ queryKey: ["status"] })} />
      )}
    </div>
  );
}

function Shell({ onSignedOut }: { onSignedOut: () => void }) {
  const queryClient = useQueryClient();
  const entries = useEntries();

  // Anything mid-pipeline or waiting on a decision belongs in the Downloads badge.
  const pending = (entries.data ?? []).filter((e) =>
    ["downloading", "extracting", "extracted", "installing"].includes(e.state.kind),
  ).length;

  // Two kinds of signal. `game-state` carries one game's new state and is patched into
  // the cache directly, refetching instead would read the whole catalogue from the
  // server several times a second, which left progress bars lagging so far behind that
  // they appeared frozen. `library-changed` is for structural changes and does refetch.
  useEffect(() => {
    if (isMockBackend) return;
    const unlisteners: Array<() => void> = [];

    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");

      unlisteners.push(
        await listen<{ gameId: number; state: LibraryEntry["state"] }>("game-state", (e) => {
          const { gameId, state } = e.payload;
          queryClient.setQueryData<LibraryEntry[]>(["entries"], (current) =>
            current?.map((entry) =>
              entry.game.id === gameId ? { ...entry, state } : entry,
            ),
          );
        }),
      );

      unlisteners.push(
        await listen("library-changed", () => {
          void queryClient.invalidateQueries({ queryKey: ["entries"] });
        }),
      );
    })();

    return () => unlisteners.forEach((off) => off());
  }, [queryClient]);

  return (
    <div className="flex min-h-0 flex-1">
      <Sidebar downloadCount={pending} />
      <main className="flex min-h-0 min-w-0 flex-1 flex-col">
        <Routes>
          <Route path="/" element={<LibraryView />} />
          <Route path="/downloads" element={<DownloadsView />} />
          <Route path="/installed" element={<InstalledView />} />
          <Route path="/settings" element={<SettingsView onSignedOut={onSignedOut} />} />
        </Routes>
      </main>
    </div>
  );
}

function Splash() {
  return (
    <div className="flex flex-1 items-center justify-center">
      <div className="h-6 w-6 animate-spin rounded-full border-2 border-default-300 border-t-primary" />
    </div>
  );
}

/**
 * Shown while the server cannot be reached.
 *
 * Deliberately a banner and not a blocking screen. Installed games live on this machine
 * and still launch, so the app carries on working from what it cached and says plainly
 * which parts cannot work until the server is back.
 */
function OfflineBanner({ serverUrl }: { serverUrl: string | null }) {
  return (
    <div
      role="status"
      className="flex shrink-0 items-center justify-center gap-2 bg-warning/15 px-4 py-1.5 text-[11px] text-warning-600"
    >
      <Icon name="offline" className="h-3.5 w-3.5 shrink-0" />
      <span>
        Can't reach {serverUrl ? shortHost(serverUrl) : "your server"}. Your installed
        games still work; downloads and new artwork will not until it is back.
      </span>
    </div>
  );
}

/** Just the host, so a long address does not push the explanation off the banner. */
function shortHost(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

/** Visible reminder that this is fixture data, so a screenshot is never mistaken for real. */
function MockBanner() {
  return (
    <div className="shrink-0 bg-warning/15 px-4 py-1 text-center text-[11px] text-warning-600">
      Running outside Tauri, showing fixture data
    </div>
  );
}
