import { useCallback, useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Route, Routes, useNavigate } from "react-router-dom";
import { GamepadOverlay } from "@/components/GamepadOverlay";
import { Icon } from "@/components/Icon";
import { ResizeHandles } from "@/components/ResizeHandles";
import { Sidebar } from "@/components/Sidebar";
import { TitleBar } from "@/components/TitleBar";
import { UpdateBanner } from "@/components/UpdateBanner";
import { WinePrompt } from "@/components/WinePrompt";
import { isMockBackend } from "@/lib/backend";
import { useGamepad } from "@/lib/useGamepad";
import { useCouch } from "@/state/couch";
import type { LibraryEntry } from "@/types";
import { DownloadsView } from "@/views/DownloadsView";
import { InstalledView } from "@/views/InstalledView";
import { LibraryView } from "@/views/LibraryView";
import { SettingsView } from "@/views/SettingsView";
import { WelcomeView } from "@/views/WelcomeView";
import { useAppSettings, useEntries, useStatus } from "@/lib/queries";

export function App() {
  const queryClient = useQueryClient();
  const navigate = useNavigate();

  const status = useStatus();

  // Don't flash the wizard while a stored session is still being restored.
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
      // The event may have fired before this listener attached; this is the backstop.
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

  // Once the server answers again, everything fetched from cache while it was down is worth re-reading.
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
      <UpdateBanner />

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
  const navigate = useNavigate();
  const entries = useEntries();
  const settings = useAppSettings();
  const couch = useCouch((state) => state.couch);

  // Bindings mount once here; what a button does is resolved at press time from what is on screen.
  useGamepad();

  // A root-element attribute so the layout switch lives in CSS, not a prop threaded everywhere.
  useEffect(() => {
    document.documentElement.dataset.couch = couch ? "true" : "false";
  }, [couch]);

  // Tailwind and HeroUI both key off a `dark` class on the root element.
  const theme = settings.data?.theme ?? "dark";
  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      const dark = theme === "dark" || (theme === "system" && media.matches);
      document.documentElement.classList.toggle("dark", dark);
      document.documentElement.classList.toggle("light", !dark);
    };
    apply();
    // Only worth listening to while actually following the system.
    if (theme !== "system") return;
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [theme]);

  // Tray items navigate the open window rather than reloading it and losing in-flight queries.
  useEffect(() => {
    if (isMockBackend) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      const off = await listen<string>("navigate", (event) => navigate(event.payload));
      if (cancelled) off();
      else unlisten = off;
    })();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [navigate]);

  // Anything mid-pipeline or awaiting a decision counts towards the Downloads badge.
  const pending = (entries.data ?? []).filter((e) =>
    ["downloading", "extracting", "extracted", "installing"].includes(e.state.kind),
  ).length;

  // `game-state` is patched into the cache directly (refetching the catalogue that often
  // froze progress bars); `library-changed` is structural and does refetch.
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
      <WinePrompt />
      <Sidebar downloadCount={pending} />
      <main className="flex min-h-0 min-w-0 flex-1 flex-col">
        <GamepadOverlay />
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

/** Shown while the server is unreachable. A banner, not a blocking screen: cached data
 * and installed games still work. */
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

/** Fixture-data banner, so a screenshot is never mistaken for the real thing. */
function MockBanner() {
  return (
    <div className="shrink-0 bg-warning/15 px-4 py-1 text-center text-[11px] text-warning-600">
      Running outside Tauri, showing fixture data
    </div>
  );
}
