import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Icon } from "@/components/Icon";
import { updateChannelNote, useUpdate } from "@/components/UpdateBanner";
import { useLibraryRoots } from "@/components/RootChooser";
import {
  backend,
  isMockBackend,
  type PrefixEntry,
  type Theme,
  type WineProgress,
  type WineVariant,
} from "@/lib/backend";
import { formatBytes, formatSpeed } from "@/lib/format";
import { messageOf } from "@/lib/errors";
import { useAppSettings, useStatus } from "@/lib/queries";
import { useCouch } from "@/state/couch";

/** Windows runs its own programs; none of the compatibility machinery applies there. */
const isWindows =
  typeof navigator !== "undefined" && /win/i.test(navigator.platform || navigator.userAgent);

/** Which pane of Settings is showing. */
type TabId =
  | "account"
  | "library"
  | "interface"
  | "compatibility"
  | "diagnostics"
  | "about";

const TAB_KEY = "gameyfin.settings.tab";

/**
 * The panes, in the order they appear.
 *
 * A list rather than a hand-written row of buttons so adding a pane means adding one
 * entry here and one branch below, which is the point of splitting Settings up: it was a
 * single scrolling column, and it is going to keep growing.
 */
const TABS: Array<{ id: TabId; label: string; hideOnWindows?: boolean }> = [
  { id: "account", label: "Account" },
  { id: "library", label: "Library" },
  { id: "interface", label: "Interface" },
  // Wine and the installer memory cap only exist because Windows software has to be
  // translated; on Windows itself there is nothing here to configure.
  { id: "compatibility", label: "Compatibility", hideOnWindows: true },
  { id: "diagnostics", label: "Diagnostics" },
  { id: "about", label: "About" },
];

export function SettingsView({ onSignedOut }: { onSignedOut: () => void }) {
  const tabs = TABS.filter((tab) => !(isWindows && tab.hideOnWindows));

  const [tab, setTab] = useState<TabId>(() => {
    try {
      const stored = localStorage.getItem(TAB_KEY);
      // A pane that no longer exists, or one hidden on this platform, must not leave the
      // view blank.
      if (tabs.some((t) => t.id === stored)) return stored as TabId;
    } catch {
      // Private windows and blocked site data both throw here; the default is fine.
    }
    return "account";
  });

  function select(next: TabId) {
    setTab(next);
    try {
      localStorage.setItem(TAB_KEY, next);
    } catch {
      // A remembered pane is a convenience, not a requirement.
    }
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div
        role="tablist"
        aria-label="Settings"
        className="flex shrink-0 items-center gap-1 border-b border-default-200/60 px-6"
      >
        {tabs.map((item) => (
          <button
            key={item.id}
            type="button"
            role="tab"
            aria-selected={tab === item.id}
            onClick={() => select(item.id)}
            className={`-mb-px border-b-2 px-3 py-2.5 text-xs font-medium transition-colors ${
              tab === item.id
                ? "border-primary text-primary"
                : "border-transparent text-foreground/50 hover:text-foreground"
            }`}
          >
            {item.label}
          </button>
        ))}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
        <div className="mx-auto flex max-w-2xl flex-col gap-5">
          {tab === "account" && <AccountSection onSignedOut={onSignedOut} />}
          {tab === "library" && (
            <>
              <RootsSection />
              <DownloadSection />
              <ExtractionSection />
            </>
          )}
          {tab === "interface" && (
            <>
              <AppearanceSection />
              <NotificationSection />
              <WindowSection />
              <GamepadSection />
            </>
          )}
          {tab === "compatibility" && (
            <>
              <WineSection />
              <UmuSection />
              <CompatibilitySection />
              <PrefixSection />
            </>
          )}
          {tab === "diagnostics" && <DiagnosticsSection />}
          {tab === "about" && <AboutSection />}
        </div>
      </div>
    </div>
  );
}

function AccountSection({ onSignedOut }: { onSignedOut: () => void }) {
  const status = useStatus();

  // Three states, not two. "Offline" is the server not answering, which leaves the
  // session intact and the cached library readable, and saying "Disconnected" for it
  // would suggest the user has been signed out when they have not.
  const connection = status.data?.offline
    ? { value: "Offline, showing your cached library", tone: "bad" as const }
    : status.data?.authenticated
      ? { value: "Connected", tone: "good" as const }
      : { value: "Disconnected", tone: "bad" as const };

  return (
    <Section title="Account">
      <Row label="Server" value={status.data?.serverUrl ?? "Not configured"} />
      <Row label="Signed in as" value={status.data?.username ?? "Not signed in"} />
      <Row label="Status" value={connection.value} tone={connection.tone} />
      <div className="pt-1">
        <button
          type="button"
          onClick={async () => {
            await backend.signOut();
            onSignedOut();
          }}
          className="rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/70 transition-colors hover:border-danger/40 hover:bg-danger/10 hover:text-danger"
        >
          Sign out
        </button>
      </div>
    </Section>
  );
}

function AboutSection() {
  const update = useUpdate();
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const settings = useAppSettings();
  const status = update.data;

  async function install() {
    setBusy(true);
    setError(null);
    try {
      setMessage(await backend.installUpdate());
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Section title="About">
      <Row label="Version" value={status?.currentVersion ?? "0.1.0"} />
      <Row
        label="Latest release"
        value={
          update.isLoading
            ? "Checking…"
            : status?.error
              ? "Could not check"
              : (status?.latestVersion ?? "Unknown")
        }
        tone={status?.available ? "bad" : undefined}
      />

      <div className="flex flex-wrap gap-2 pt-1">
        <button
          type="button"
          disabled={update.isFetching}
          onClick={() => void update.refetch()}
          className="rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/70 transition-colors hover:bg-default-100 disabled:opacity-50"
        >
          {update.isFetching ? "Checking…" : "Check now"}
        </button>
        {status?.available && status.canInstall && (
          <button
            type="button"
            disabled={busy}
            onClick={() => void install()}
            className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-primary-600 disabled:opacity-50"
          >
            {busy ? "Updating…" : `Update to ${status.latestVersion}`}
          </button>
        )}
        {status && (
          <button
            type="button"
            onClick={() => void backend.openPath(status.releaseUrl)}
            className="rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/70 transition-colors hover:bg-default-100"
          >
            Release notes
          </button>
        )}
      </div>

      {message && <p className="text-[11px] text-foreground/60">{message}</p>}
      {error && (
        <p role="alert" className="text-[11px] text-danger">
          {error}
        </p>
      )}

      <Check
        label="Check for updates at startup"
        hint="One request to GitHub when the app opens. Nothing is downloaded until you ask."
        checked={settings.data?.checkForUpdates ?? true}
        onChange={async (next) => {
          await backend.setUpdateChecking(next);
          await settings.refetch();
        }}
      />

      {status && (
        <p className="text-[11px] leading-relaxed text-foreground/45">
          {updateChannelNote(status)}
        </p>
      )}
    </Section>
  );
}

/** What a finished download should do next. */
function DownloadSection() {
  const settings = useAppSettings();

  return (
    <Section title="Downloads">
      <Check
        label="Install automatically when a download finishes"
        hint="Unpacks the download and moves the game into your installations folder without asking. A download that turns out to contain a setup program still stops and waits, because a setup wizard asks questions this cannot answer for you."
        checked={settings.data?.autoInstall ?? false}
        onChange={async (next) => {
          await backend.setAutoInstall(next);
          await settings.refetch();
        }}
      />
    </Section>
  );
}

/** Desktop notifications. */
function NotificationSection() {
  const settings = useAppSettings();

  async function save(changes: {
    transfers?: boolean;
    failures?: boolean;
    updates?: boolean;
  }) {
    const current = settings.data;
    if (!current) return;
    await backend.setNotificationOptions(
      changes.transfers ?? current.notifyTransfers,
      changes.failures ?? current.notifyFailures,
      changes.updates ?? current.notifyUpdates,
    );
    await settings.refetch();
  }

  return (
    <Section title="Notifications">
      <Check
        label="Downloads and installs"
        hint="When a download is ready to install, and when a game is ready to play."
        checked={settings.data?.notifyTransfers ?? true}
        onChange={(next) => save({ transfers: next })}
      />
      <Check
        label="Failures"
        hint="When a download, install or launch goes wrong. Shown even when the window has your attention, because the alternative is red text on a tab you are not looking at."
        checked={settings.data?.notifyFailures ?? true}
        onChange={(next) => save({ failures: next })}
      />
      <Check
        label="New versions of Gameyfin"
        checked={settings.data?.notifyUpdates ?? true}
        onChange={(next) => save({ updates: next })}
      />
      <p className="text-[11px] leading-relaxed text-foreground/45">
        Notifications are held back while you are looking at the window, apart from
        failures. Telling you what you can already see is the fastest way to make anyone
        turn them all off.
      </p>
    </Section>
  );
}

/** The tray, and what the close button does. */
function WindowSection() {
  const settings = useAppSettings();

  async function save(changes: { closeToTray?: boolean; startMinimized?: boolean }) {
    const current = settings.data;
    if (!current) return;
    await backend.setWindowOptions(
      changes.closeToTray ?? current.closeToTray,
      changes.startMinimized ?? current.startMinimized,
    );
    await settings.refetch();
  }

  return (
    <Section title="Window">
      <Check
        label="Closing the window keeps Gameyfin running"
        hint="Downloads run inside this program, so closing the window used to abandon one that might have had an hour left. With this on, the close button hides the window and the tray icon brings it back."
        checked={settings.data?.closeToTray ?? true}
        onChange={(next) => save({ closeToTray: next })}
      />
      <Check
        label="Start hidden in the tray"
        hint="For starting Gameyfin with your session without a window appearing."
        checked={settings.data?.startMinimized ?? false}
        onChange={(next) => save({ startMinimized: next })}
      />
      <Check
        label="Start Gameyfin when I log in"
        hint="Registers Gameyfin with your desktop so it starts hidden in the tray with your session. What makes downloading in the background actually work."
        checked={settings.data?.autostart ?? false}
        onChange={async (next) => {
          await backend.setAutostart(next);
          await settings.refetch();
        }}
      />
      <div className="pt-1">
        <button
          type="button"
          onClick={() => void backend.quitApp()}
          className="rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/70 transition-colors hover:border-danger/40 hover:bg-danger/10 hover:text-danger"
        >
          Quit Gameyfin
        </button>
      </div>
    </Section>
  );
}

/** Controller support. */
function GamepadSection() {
  const settings = useAppSettings();
  const connected = useCouch((state) => state.connected);
  const name = useCouch((state) => state.name);
  const toggleHelp = useCouch((state) => state.toggleHelp);

  async function save(changes: {
    enabled?: boolean;
    deadzone?: number;
    couchModeAuto?: boolean;
  }) {
    const current = settings.data;
    if (!current) return;
    await backend.setGamepadOptions(
      changes.enabled ?? current.gamepadEnabled,
      changes.deadzone ?? current.gamepadDeadzone,
      changes.couchModeAuto ?? current.couchModeAuto,
    );
    await settings.refetch();
  }

  const deadzone = settings.data?.gamepadDeadzone ?? 0.25;

  return (
    <Section title="Controller">
      <Row
        label="Detected"
        value={connected ? (name ?? "A controller") : "None connected"}
        tone={connected ? "good" : undefined}
      />
      <Check
        label="Read connected controllers"
        checked={settings.data?.gamepadEnabled ?? true}
        onChange={(next) => save({ enabled: next })}
      />
      <Check
        label="Switch to the large layout when a controller connects"
        hint="Bigger text and fewer, larger covers, for reading from a sofa. You can switch back from the controller overlay at any time."
        checked={settings.data?.couchModeAuto ?? true}
        onChange={(next) => save({ couchModeAuto: next })}
      />

      <label className="pt-1 text-xs text-foreground/55" htmlFor="gamepad-deadzone">
        Stick dead zone: {Math.round(deadzone * 100)}%
      </label>
      <input
        id="gamepad-deadzone"
        type="range"
        min={5}
        max={60}
        step={5}
        value={Math.round(deadzone * 100)}
        onChange={(e) => void save({ deadzone: Number(e.target.value) / 100 })}
        className="w-full accent-primary"
      />
      <p className="text-[11px] leading-relaxed text-foreground/45">
        How far a stick must move before it counts. Raise this if the selection drifts on
        its own; a worn stick rests slightly off centre, and without a dead zone that
        reads as a direction being held down forever.
      </p>

      <div className="pt-1">
        <button
          type="button"
          onClick={toggleHelp}
          className="rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/70 transition-colors hover:bg-default-100"
        >
          Show the button map
        </button>
      </div>
    </Section>
  );
}

/** Per-title Proton fixes. */
function UmuSection() {
  const settings = useAppSettings();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const status = useQuery({ queryKey: ["umu"], queryFn: () => backend.umuStatus() });

  async function refresh() {
    setBusy(true);
    setError(null);
    try {
      await backend.refreshUmuDatabase();
      await status.refetch();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Section title="Game fixes">
      <Row
        label="Known games"
        value={
          status.isLoading
            ? "…"
            : status.data?.entries
              ? `${status.data.entries.toLocaleString()} titles`
              : "Not downloaded yet"
        }
        tone={status.data?.entries ? "good" : undefined}
      />
      <Check
        label="Apply per-title Proton fixes"
        hint="Looks each game up in the umu database and passes its id to Proton, so workarounds written for that specific game are applied. Matched by Steam AppID where your server knows one, and by title otherwise."
        checked={settings.data?.umuFixes ?? true}
        onChange={async (next) => {
          await backend.setUmuFixes(next);
          await settings.refetch();
        }}
      />
      <div className="pt-1">
        <button
          type="button"
          disabled={busy}
          onClick={() => void refresh()}
          className="rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/70 transition-colors hover:bg-default-100 disabled:opacity-50"
        >
          {busy ? "Downloading…" : "Update the list"}
        </button>
      </div>
      {error && (
        <p role="alert" className="text-[11px] text-danger">
          {error}
        </p>
      )}
      <p className="text-[11px] leading-relaxed text-foreground/45">
        Refreshed automatically once a day. A game that is not in the list runs exactly as
        it would with this turned off.
      </p>
    </Section>
  );
}

/** One prefix per game, rather than the all-or-nothing button. */
function PrefixSection() {
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState<PrefixEntry | null>(null);
  const prefixes = useQuery({ queryKey: ["prefix-list"], queryFn: () => backend.listPrefixes() });

  async function run(action: () => Promise<void>) {
    setError(null);
    try {
      await action();
      await prefixes.refetch();
    } catch (e) {
      setError(messageOf(e));
    }
  }

  const rows = prefixes.data ?? [];

  return (
    <Section title="Compatibility prefixes">
      {rows.length === 0 ? (
        <p className="text-[11px] text-foreground/45">
          {prefixes.isLoading
            ? "…"
            : "None yet. One is created the first time a Windows game runs."}
        </p>
      ) : (
        <div className="flex flex-col gap-2">
          {rows.map((prefix) => (
            <div
              key={prefix.gameId}
              className="rounded-lg border border-default-200 bg-content2 p-2.5"
            >
              <div className="flex items-baseline justify-between gap-3">
                <span className="truncate text-xs text-foreground" title={prefix.path}>
                  {prefix.title ?? `Game ${prefix.gameId}`}
                </span>
                <span className="shrink-0 text-[11px] text-foreground/45">
                  {formatBytes(prefix.bytes)}
                </span>
              </div>
              {/* A prefix whose game has gone is exactly the kind worth reclaiming, so
                  it is called out rather than quietly listed by its id. */}
              {!prefix.title && (
                <p className="mt-0.5 text-[11px] text-foreground/45">
                  No longer in your library.
                </p>
              )}
              <div className="mt-2 flex flex-wrap gap-1.5">
                <SmallButton onClick={() => void run(() => backend.openPrefixTool(prefix.gameId, "winecfg"))}>
                  Wine settings
                </SmallButton>
                <SmallButton onClick={() => void run(() => backend.openPrefixTool(prefix.gameId, "regedit"))}>
                  Registry
                </SmallButton>
                <SmallButton onClick={() => void run(() => backend.openPrefixTool(prefix.gameId, "explorer"))}>
                  Browse C:
                </SmallButton>
                <SmallButton danger onClick={() => setConfirming(prefix)}>
                  Delete
                </SmallButton>
              </div>
            </div>
          ))}
        </div>
      )}

      {error && (
        <p role="alert" className="text-[11px] leading-relaxed text-danger">
          {error}
        </p>
      )}

      <p className="text-[11px] leading-relaxed text-foreground/45">
        A prefix is the fake Windows a game runs inside. Deleting one is safe in that it
        is rebuilt on the next launch, but anything the game stored inside it goes too.
      </p>

      {confirming && (
        <ConfirmDialog
          title={`Delete the prefix for ${confirming.title ?? `game ${confirming.gameId}`}?`}
          body={
            <>
              It is rebuilt the next time the game runs, so this is a good way to recover
              from one that has broken. Anything the game saved <em>inside</em> the prefix
              rather than in its own folder is removed with it, which for some Windows
              games includes save files.
            </>
          }
          confirmLabel="Delete the prefix"
          onConfirm={() => {
            const target = confirming;
            setConfirming(null);
            void run(() => backend.deletePrefix(target.gameId));
          }}
          onCancel={() => setConfirming(null)}
        />
      )}
    </Section>
  );
}

function SmallButton({
  children,
  onClick,
  danger,
}: {
  children: React.ReactNode;
  onClick: () => void;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`rounded-lg border border-default-200 px-2.5 py-1 text-[11px] transition-colors ${
        danger
          ? "text-foreground/60 hover:border-danger/40 hover:bg-danger/10 hover:text-danger"
          : "text-foreground/70 hover:bg-default-100"
      }`}
    >
      {children}
    </button>
  );
}

/** A labelled checkbox with an optional explanation underneath. */
function Check({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (next: boolean) => void | Promise<void>;
}) {
  return (
    <div>
      <label className="flex cursor-pointer items-start gap-2 text-xs text-foreground/80">
        <input
          type="checkbox"
          checked={checked}
          onChange={(e) => void onChange(e.target.checked)}
          className="mt-0.5 h-3.5 w-3.5 shrink-0 accent-primary"
        />
        <span>{label}</span>
      </label>
      {hint && <p className="mt-0.5 pl-[1.375rem] text-[11px] leading-relaxed text-foreground/45">{hint}</p>}
    </div>
  );
}

/** The games folders, and which one is the default. */
function RootsSection() {
  const queryClient = useQueryClient();
  const roots = useLibraryRoots();
  const [error, setError] = useState<string | null>(null);
  const [removing, setRemoving] = useState<string | null>(null);

  async function run(action: () => Promise<void>) {
    setError(null);
    try {
      await action();
      await roots.refetch();
      // The wizard and the status card both read the primary folder.
      await queryClient.invalidateQueries({ queryKey: ["status"] });
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
    } catch (e) {
      setError(messageOf(e));
    }
  }

  async function add() {
    const chosen = await backend.pickFolder();
    if (chosen) await run(() => backend.addLibraryRoot(chosen));
  }

  const list = roots.data ?? [];

  return (
    <Section title="Games folders">
      {list.length === 0 && !roots.isLoading && (
        <p className="text-[11px] text-foreground/45">
          No folder yet. Add one and downloads will go there.
        </p>
      )}

      <div className="flex flex-col gap-2">
        {list.map((root) => (
          <div
            key={root.path}
            className="rounded-lg border border-default-200 bg-content2 p-2.5"
          >
            <div className="flex items-baseline justify-between gap-3">
              <code
                className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground"
                title={root.path}
              >
                {root.path}
              </code>
              {root.isDefault && (
                <span className="shrink-0 rounded bg-primary/15 px-1.5 py-0.5 text-[10px] text-primary">
                  Default
                </span>
              )}
            </div>
            <p className="mt-0.5 text-[11px] text-foreground/45">
              {!root.exists
                ? "This folder is missing. Games in it will not be found until it is back."
                : root.freeBytes === null
                  ? "Free space unknown"
                  : `${formatBytes(root.freeBytes)} free`}
            </p>
            <div className="mt-2 flex flex-wrap gap-1.5">
              {!root.isDefault && (
                <SmallButton
                  onClick={() => void run(() => backend.setDefaultLibraryRoot(root.path))}
                >
                  Make default
                </SmallButton>
              )}
              <SmallButton
                onClick={() => void backend.openLibraryFolder("installations", root.path)}
              >
                Open
              </SmallButton>
              {list.length > 1 && (
                <SmallButton danger onClick={() => setRemoving(root.path)}>
                  Remove
                </SmallButton>
              )}
            </div>
          </div>
        ))}
      </div>

      <div className="pt-1">
        <button
          type="button"
          onClick={() => void add()}
          className="rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/70 transition-colors hover:bg-default-100"
        >
          Add a folder…
        </button>
      </div>

      {error && (
        <p role="alert" className="text-[11px] leading-relaxed text-danger">
          {error}
        </p>
      )}

      <p className="text-[11px] leading-relaxed text-foreground/45">
        Downloads go to <code className="text-foreground/60">Gameyfin/Downloads</code> and
        installs to <code className="text-foreground/60">Gameyfin/Installations</code>{" "}
        inside each of these. With more than one folder you are asked which to use when a
        download starts, with the free space on each shown.
      </p>

      {removing && (
        <ConfirmDialog
          title={`Stop using ${removing}?`}
          body={
            <>
              Gameyfin will forget this folder and stop looking in it. <em>Nothing on
              disk is deleted</em>, and adding it back later finds the games again.
            </>
          }
          confirmLabel="Remove from the list"
          onConfirm={() => {
            const target = removing;
            setRemoving(null);
            void run(() => backend.removeLibraryRoot(target));
          }}
          onCancel={() => setRemoving(null)}
        />
      )}
    </Section>
  );
}

/** The archive password and the executables never worth offering. */
function ExtractionSection() {
  const settings = useAppSettings();
  const [password, setPassword] = useState<string | null>(null);
  const [ignored, setIgnored] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const currentPassword = password ?? settings.data?.extractionPassword ?? "";
  const currentIgnored = ignored ?? (settings.data?.ignoredExecutables ?? []).join("\n");

  async function save() {
    setError(null);
    try {
      await backend.setExtractionOptions(
        currentPassword || null,
        currentIgnored.split("\n"),
      );
      await settings.refetch();
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } catch (e) {
      setError(messageOf(e));
    }
  }

  return (
    <Section title="Extraction">
      <label className="text-xs text-foreground/55" htmlFor="extraction-password">
        Archive password
      </label>
      <input
        id="extraction-password"
        type="password"
        value={currentPassword}
        onChange={(e) => setPassword(e.target.value)}
        spellCheck={false}
        placeholder="None"
        className="rounded-lg border border-default-200 bg-content2 px-3 py-2 font-mono text-xs outline-none transition-colors focus:border-primary"
      />
      <p className="text-[11px] leading-relaxed text-foreground/45">
        Tried automatically when an archive turns out to be encrypted. Stored in the app's
        settings file, which is readable only by you, alongside your session. It is a
        convenience for a library that uses one password throughout, not a secret store.
      </p>

      <label className="pt-2 text-xs text-foreground/55" htmlFor="ignored-executables">
        Never offer these executables
      </label>
      <textarea
        id="ignored-executables"
        rows={6}
        value={currentIgnored}
        onChange={(e) => setIgnored(e.target.value)}
        spellCheck={false}
        className="rounded-lg border border-default-200 bg-content2 px-3 py-2 font-mono text-[11px] outline-none transition-colors focus:border-primary"
      />
      <p className="text-[11px] leading-relaxed text-foreground/45">
        One per line, matched anywhere in the file name. Redistributables and crash
        handlers ship beside a game in numbers, and on a large install the actual launcher
        is easily lost among them.
      </p>

      <div className="pt-1">
        <button
          type="button"
          onClick={() => void save()}
          className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-primary-600"
        >
          {saved ? "Saved" : "Save"}
        </button>
      </div>
      {error && (
        <p role="alert" className="text-[11px] text-danger">
          {error}
        </p>
      )}
    </Section>
  );
}

/** Palette, and starting with the session. */
function AppearanceSection() {
  const settings = useAppSettings();
  const [error, setError] = useState<string | null>(null);

  async function change(theme: Theme) {
    setError(null);
    try {
      await backend.setTheme(theme);
      await settings.refetch();
    } catch (e) {
      setError(messageOf(e));
    }
  }

  return (
    <Section title="Appearance">
      <label className="text-xs text-foreground/55" htmlFor="theme">
        Theme
      </label>
      <select
        id="theme"
        value={settings.data?.theme ?? "dark"}
        onChange={(e) => void change(e.target.value as Theme)}
        className="rounded-lg border border-default-200 bg-content2 px-3 py-2 text-sm outline-none focus:border-primary"
      >
        <option value="dark">Dark</option>
        <option value="light">Light</option>
        <option value="system">Match my system</option>
      </select>
      {error && (
        <p role="alert" className="text-[11px] text-danger">
          {error}
        </p>
      )}
    </Section>
  );
}

/** The Wine runtime the app downloads and keeps up to date. */
function WineSection() {
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState<"install" | "remove" | null>(null);
  const [progress, setProgress] = useState<WineProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  const status = useQuery({
    queryKey: ["wine-status"],
    queryFn: () => backend.wineStatus(),
    // The release lookup hits the network, so this is not something to refetch on every
    // window focus.
    staleTime: 5 * 60 * 1000,
  });

  const settings = useAppSettings();
  const variant: WineVariant = settings.data?.wineVariant ?? "staging-wow64";
  const installed = status.data?.installed ?? null;
  const latest = status.data?.latest ?? null;
  const updatable =
    installed && latest && (installed.version !== latest.version || installed.variant !== latest.variant);

  useEffect(() => {
    // Only subscribed while a download is actually running, so an idle settings screen
    // holds no listener.
    if (busy !== "install" || isMockBackend) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      const off = await listen<WineProgress>("wine-progress", (event) => {
        if (!cancelled) setProgress(event.payload);
      });
      // The download can finish before this attaches; dropping the listener immediately
      // in that case avoids leaking it for the life of the view.
      if (cancelled) off();
      else unlisten = off;
    })();

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [busy]);

  async function run(action: "install" | "remove") {
    setBusy(action);
    setError(null);
    setProgress(null);
    try {
      if (action === "install") await backend.installWine();
      else await backend.removeWine();
      await status.refetch();
      // The install options screen greys itself out on a missing runtime, so it has to
      // be told that one now exists.
      await queryClient.invalidateQueries({ queryKey: ["install-options"] });
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(null);
      setProgress(null);
    }
  }

  async function changeVariant(next: WineVariant) {
    await backend.setWineVariant(next);
    await settings.refetch();
    await status.refetch();
  }

  return (
    <Section title="Wine">
      <Row
        label="Installed"
        value={installed ? `${installed.version} (${labelFor(installed.variant)})` : "Not installed"}
        tone={installed ? "good" : undefined}
      />
      <Row
        label="Latest available"
        value={
          status.isLoading
            ? "Checking…"
            : latest
              ? latest.version
              : "Could not check, no connection"
        }
        tone={updatable ? "bad" : undefined}
      />

      {busy === "install" && (
        <div className="flex flex-col gap-1 pt-1">
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-default-200">
            <div
              className="h-full bg-primary transition-[width]"
              style={{
                width: progress?.totalBytes
                  ? `${Math.round((progress.receivedBytes / progress.totalBytes) * 100)}%`
                  : "0%",
              }}
            />
          </div>
          <p className="text-[11px] text-foreground/45">
            {progress
              ? `${formatBytes(progress.receivedBytes)} of ${formatBytes(progress.totalBytes)} at ${formatSpeed(progress.bytesPerSecond)}`
              : "Starting download…"}
          </p>
        </div>
      )}

      <div className="flex flex-wrap gap-2 pt-1">
        <button
          type="button"
          disabled={busy !== null}
          onClick={() => void run("install")}
          className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground disabled:opacity-50"
        >
          {busy === "install"
            ? "Downloading…"
            : !installed
              ? `Download Wine${latest ? ` (${formatBytes(latest.sizeBytes)})` : ""}`
              : updatable
                ? `Update to ${latest?.version}`
                : "Redownload"}
        </button>
        {installed && (
          <button
            type="button"
            disabled={busy !== null}
            onClick={() => void run("remove")}
            className="rounded-lg border border-default-200 px-3 py-1.5 text-xs font-medium disabled:opacity-50"
          >
            {busy === "remove" ? "Removing…" : "Remove"}
          </button>
        )}
      </div>

      {error && <p className="text-[11px] leading-relaxed text-danger">{error}</p>}

      <label className="pt-2 text-xs text-foreground/55" htmlFor="wine-variant">
        Build
      </label>
      <select
        id="wine-variant"
        value={variant}
        onChange={(e) => void changeVariant(e.target.value as WineVariant)}
        className="rounded-lg border border-default-200 bg-content2 px-3 py-2 text-sm outline-none focus:border-primary"
      >
        <option value="staging-wow64">Wine-Staging, WoW64 (recommended)</option>
        <option value="staging">Wine-Staging, 32-bit libraries</option>
      </select>
      <p className="text-[11px] leading-relaxed text-foreground/45">
        Gameyfin downloads its own Wine so every install works the same way, with nothing
        to install on your system and no administrator rights needed. Both builds run
        32-bit and 64-bit Windows programs. The recommended one needs no 32-bit system
        libraries, which is what lets it work identically inside a Flatpak. Switch to the
        other only if an installer misbehaves. Outside a Flatpak that build needs your
        distribution's 32-bit libraries, and inside one it needs the i386 compatibility
        runtime.
      </p>
      {installed && (
        <p className="text-[11px] leading-relaxed text-foreground/45">
          Removing Wine leaves your game prefixes and saves untouched.
        </p>
      )}
    </Section>
  );
}

/** Human-readable name for a build. */
function labelFor(variant: WineVariant): string {
  return variant === "staging" ? "32-bit libraries" : "WoW64";
}

/** Options that only matter when running Windows software. */
function CompatibilitySection() {
  const settings = useAppSettings();
  const [limit, setLimit] = useState<number | null>(null);
  const current = limit ?? settings.data?.installerMemoryLimitMb ?? 3072;

  async function change(next: number) {
    setLimit(next);
    await backend.setInstallerMemoryLimit(next);
  }

  return (
    <Section title="Compatibility">
      <label className="text-xs text-foreground/55" htmlFor="installer-memory">
        Installer memory limit
      </label>
      <select
        id="installer-memory"
        value={current}
        onChange={(e) => void change(Number(e.target.value))}
        className="rounded-lg border border-default-200 bg-content2 px-3 py-2 text-sm outline-none focus:border-primary"
      >
        <option value={0}>No limit</option>
        <option value={3072}>3 GB (recommended)</option>
        <option value={4096}>4 GB</option>
        <option value={6144}>6 GB</option>
      </select>
      <p className="text-[11px] leading-relaxed text-foreground/45">
        Some repack installers use a decompression library that hangs, spinning one CPU
        core with the progress bar frozen, when it is offered more than 2 GB of
        contiguous memory. Capping the installer avoids it. Raise this only if an
        installer runs out of memory; going below 3 GB makes installers fail outright.
      </p>
    </Section>
  );
}

const LOG_LEVELS = [
  { key: "error", label: "Error" },
  { key: "warn", label: "Warning" },
  { key: "info", label: "Info" },
  { key: "debug", label: "Debug" },
];

function DiagnosticsSection() {
  const logs = useQuery({ queryKey: ["log-dir"], queryFn: () => backend.logDirectory() });
  const settings = useAppSettings();
  const cacheSize = useQuery({ queryKey: ["image-cache"], queryFn: () => backend.imageCacheSize() });
  const configDir = useQuery({ queryKey: ["config-dir"], queryFn: () => backend.configDirectory() });
  const [level, setLevel] = useState<string | null>(null);
  const [levelError, setLevelError] = useState<string | null>(null);

  const current = level ?? settings.data?.logLevel ?? "info";

  async function change(next: string) {
    setLevel(next);
    setLevelError(null);
    try {
      await backend.setLogLevel(next);
    } catch (e) {
      setLevelError(messageOf(e));
    }
  }

  return (
    <Section title="Diagnostics">
      <label className="text-xs text-foreground/55" htmlFor="log-level">
        Log detail
      </label>
      <select
        id="log-level"
        value={current}
        onChange={(e) => void change(e.target.value)}
        className="rounded-lg border border-default-200 bg-content2 px-3 py-2 text-sm outline-none focus:border-primary"
      >
        {LOG_LEVELS.map((option) => (
          <option key={option.key} value={option.key}>
            {option.label}
          </option>
        ))}
      </select>
      <p className="text-[11px] text-foreground/45">
        Takes effect immediately, with no restart needed. Turn this up to Debug before
        reproducing a problem, then attach the log.
      </p>
      {levelError && (
        <p role="alert" className="text-xs text-danger">
          {levelError}
        </p>
      )}

      <div className="mt-2 flex items-center justify-between gap-4">
        <div className="min-w-0">
          <p className="text-xs text-foreground/55">Artwork cache</p>
          <p className="text-[11px] text-foreground/45">
            {cacheSize.data === undefined
              ? "…"
              : `${formatBytes(cacheSize.data)}, and it cleans itself as it grows`}
          </p>
        </div>
        <button
          type="button"
          onClick={async () => {
            await backend.clearImageCache();
            await cacheSize.refetch();
          }}
          className="shrink-0 rounded-lg border border-default-200 px-3 py-1.5 text-xs text-foreground/70 transition-colors hover:bg-default-100"
        >
          Clear
        </button>
      </div>

      <PathRow
        label="App data"
        hint="Settings, session and local records for this installation."
        path={configDir.data}
      />

      <PathRow
        label="Log files"
        hint="One file per day. Include the most recent when reporting a problem."
        path={logs.data}
      />
    </Section>
  );
}

/** A filesystem path with an Open button, and optionally a destructive action. */
function PathRow({
  label,
  hint,
  path,
  action,
}: {
  label: string;
  hint: string;
  path: string | undefined;
  action?: { label: string; danger?: boolean; onClick: () => void | Promise<void> };
}) {
  return (
    <div>
      <p className="text-xs text-foreground/55">{label}</p>
      <div className="mt-1 flex gap-2">
        <code className="min-w-0 flex-1 break-all rounded-lg border border-default-200 bg-content2 px-3 py-2 font-mono text-[11px] text-foreground/70">
          {path ?? "…"}
        </code>
        <button
          type="button"
          disabled={!path}
          onClick={() => path && void backend.openPath(path)}
          className="shrink-0 self-start rounded-lg border border-default-200 px-3 py-2 text-xs text-foreground/70 transition-colors hover:bg-default-100 disabled:opacity-50"
        >
          Open folder
        </button>
        {action && (
          <button
            type="button"
            onClick={() => void action.onClick()}
            className={`shrink-0 self-start rounded-lg border px-3 py-2 text-xs transition-colors ${
              action.danger
                ? "border-default-200 text-foreground/60 hover:border-danger/40 hover:bg-danger/10 hover:text-danger"
                : "border-default-200 text-foreground/70 hover:bg-default-100"
            }`}
          >
            {action.label}
          </button>
        )}
      </div>
      <p className="mt-1 text-[11px] text-foreground/45">{hint}</p>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="rounded-xl border border-default-200 bg-content1 p-4">
      <h2 className="mb-3 flex items-center gap-2 text-xs font-semibold uppercase tracking-wide text-foreground/45">
        <Icon name="settings" className="h-3.5 w-3.5" />
        {title}
      </h2>
      <div className="flex flex-col gap-2">{children}</div>
    </section>
  );
}

function Row({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone?: "good" | "bad";
}) {
  const colour =
    tone === "good" ? "text-success" : tone === "bad" ? "text-danger" : "text-foreground";
  return (
    <div className="flex items-baseline justify-between gap-4 text-sm">
      <span className="shrink-0 text-foreground/55">{label}</span>
      <span className={`truncate ${colour}`} title={value}>
        {value}
      </span>
    </div>
  );
}
