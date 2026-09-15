import { useCallback, useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Route, Routes, useNavigate } from "react-router-dom";
import { GamepadOverlay } from "@/components/GamepadOverlay";
import { Icon } from "@/components/Icon";
import { NotificationBell } from "@/components/NotificationCenter";
import { ResizeHandles } from "@/components/ResizeHandles";
import { Sidebar } from "@/components/Sidebar";
import { TitleBar } from "@/components/TitleBar";
import { UpdateBanner } from "@/components/UpdateBanner";
import { SavePullPrompt } from "@/components/SavePullPrompt";
import { SaveSyncStatus } from "@/components/SaveSyncStatus";
import { isMockBackend } from "@/lib/backend";
import { useTauriEvent } from "@/lib/useTauriEvent";
import { useGamepad } from "@/lib/useGamepad";
import { useSetupGate } from "@/lib/useSetupGate";
import { useCouch } from "@/state/couch";
import type { LibraryEntry } from "@/types";
import { DownloadsView } from "@/views/DownloadsView";
import { InstalledView } from "@/views/InstalledView";
import { LibraryView } from "@/views/LibraryView";
import { SavesView, countNeedingAttention, useSaveOverview } from "@/views/SavesView";
import { SettingsView } from "@/views/SettingsView";
import { WelcomeView } from "@/views/WelcomeView";
import { keys, useAppSettings, useEntries, useInvalidate, useStatus } from "@/lib/queries";

export function App() {
  const queryClient = useQueryClient();
  const navigate = useNavigate();

  const status = useStatus();
  useTheme();

  // Don't flash the wizard while a stored session is still being restored.
  const [restoreSettled, setRestoreSettled] = useState(isMockBackend);
  useTauriEvent("connection-restored", () => {
    setRestoreSettled(true);
    void queryClient.invalidateQueries({ queryKey: keys.status });
  });
  // The event may have fired before the listener attached; this is the backstop.
  useEffect(() => {
    if (isMockBackend) return;
    const timer = setTimeout(() => setRestoreSettled(true), 3000);
    return () => clearTimeout(timer);
  }, []);

  const ready = restoreSettled && !status.isLoading;
  const offline = Boolean(status.data?.offline);
  // Never while offline: signing in is impossible then, and the wizard's first step offers
  // to switch servers, discarding the session and cached library.
  const needsSetup =
    ready && !offline && !(status.data?.configured && status.data?.authenticated);
  // The wizard runs to its last step, not to the moment the session becomes valid.
  const setup = useSetupGate(needsSetup);

  const onConnected = useCallback(async () => {
    setup.close();
    await queryClient.invalidateQueries();
    navigate("/");
  }, [setup, queryClient, navigate]);

  // Once the server answers again, everything fetched from cache while it was down is worth re-reading.
  const wasOffline = useRef(offline);
  useEffect(() => {
    if (wasOffline.current && !offline) void queryClient.invalidateQueries();
    wasOffline.current = offline;
  }, [offline, queryClient]);

  return (
    <div className="flex h-screen flex-col overflow-hidden bg-background text-foreground">
      <ResizeHandles />
      {/* Only once signed in: notifications lead to pages the wizard does not have. */}
      <TitleBar extra={ready && !setup.open ? <NotificationBell /> : null} />
      {isMockBackend && <MockBanner />}
      {offline && <OfflineBanner serverUrl={status.data?.serverUrl ?? null} />}
      <UpdateBanner />

      {!ready ? (
        <Splash />
      ) : setup.open ? (
        <WelcomeView onStarted={setup.engage} onComplete={onConnected} />
      ) : (
        <Shell onSignedOut={() => void queryClient.invalidateQueries({ queryKey: keys.status })} />
      )}
    </div>
  );
}

function Shell({ onSignedOut }: { onSignedOut: () => void }) {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const entries = useEntries();
  const invalidate = useInvalidate();
  const couch = useCouch((state) => state.couch);

  // Bindings mount once here; what a button does is resolved at press time from what is on screen.
  useGamepad();

  // A root-element attribute so the layout switch lives in CSS, not a prop threaded everywhere.
  useEffect(() => {
    document.documentElement.dataset.couch = couch ? "true" : "false";
  }, [couch]);

  // Tray items navigate the open window rather than reloading it and losing in-flight queries.
  useTauriEvent<string>("navigate", (route) => navigate(route));

  // Anything mid-pipeline or awaiting a decision counts towards the Downloads badge.
  const pending = (entries.data ?? []).filter((e) =>
    ["downloading", "extracting", "extracted", "installing"].includes(e.state.kind),
  ).length;

  // Saves needing a decision badge their tab too. One request covers the whole library.
  const saves = useSaveOverview("installed");
  const conflicts = countNeedingAttention(saves.data);

  // `game-state` is patched into the cache, since refetching that often freezes progress bars.
  // `library-changed` is structural, so it does refetch.
  useTauriEvent<{ gameId: number; state: LibraryEntry["state"] }>("game-state", ({ gameId, state }) => {
    queryClient.setQueryData<LibraryEntry[]>(keys.entries, (current) =>
      current?.map((entry) => (entry.game.id === gameId ? { ...entry, state } : entry)),
    );
  });
  useTauriEvent("library-changed", () => void invalidate(keys.entries));
  useTauriEvent("proton-changed", () => void invalidate(keys.proton));
  useTauriEvent("save-tool-changed", () => void invalidate(keys.saveTool, keys.saveOverviewAll));

  return (
    <div className="flex min-h-0 flex-1">
      <SavePullPrompt />
      <SaveSyncStatus />
      <Sidebar downloadCount={pending} conflictCount={conflicts} />
      <main className="flex min-h-0 min-w-0 flex-1 flex-col">
        <GamepadOverlay />
        <Routes>
          <Route path="/" element={<LibraryView />} />
          <Route path="/downloads" element={<DownloadsView />} />
          <Route path="/installed" element={<InstalledView />} />
          <Route path="/saves" element={<SavesView />} />
          <Route path="/settings" element={<SettingsView onSignedOut={onSignedOut} />} />
        </Routes>
      </main>
    </div>
  );
}

/** Tailwind keys off a `dark` class on the root element. At the top, so the splash and
 * the wizard follow the setting too. */
function useTheme() {
  const theme = useAppSettings().data?.theme ?? "dark";
  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      const dark = theme === "dark" || (theme === "system" && media.matches);
      document.documentElement.classList.toggle("dark", dark);
      document.documentElement.classList.toggle("light", !dark);
    };
    apply();
    if (theme !== "system") return;
    media.addEventListener("change", apply);
    return () => media.removeEventListener("change", apply);
  }, [theme]);
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
