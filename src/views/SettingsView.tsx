import { useEffect, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Icon } from "@/components/Icon";
import { backend, isMockBackend, type WineProgress, type WineVariant } from "@/lib/backend";
import { formatBytes, formatSpeed } from "@/lib/format";
import { messageOf } from "@/lib/errors";
import { useAppSettings, useStatus } from "@/lib/queries";

/** Windows runs its own programs; none of the compatibility machinery applies there. */
const isWindows =
  typeof navigator !== "undefined" && /win/i.test(navigator.platform || navigator.userAgent);

export function SettingsView({ onSignedOut }: { onSignedOut: () => void }) {
  const status = useStatus();

  return (
    <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
      <div className="mx-auto flex max-w-2xl flex-col gap-5">
        <Section title="Account">
          <Row label="Server" value={status.data?.serverUrl ?? "Not configured"} />
          <Row label="Signed in as" value={status.data?.username ?? "Not signed in"} />
          <Row
            label="Status"
            value={status.data?.authenticated ? "Connected" : "Disconnected"}
            tone={status.data?.authenticated ? "good" : "bad"}
          />
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

        <LibrarySection />

        {!isWindows && <WineSection />}
        {!isWindows && <CompatibilitySection />}

        <DiagnosticsSection />

        <Section title="About">
          <Row label="Version" value="0.1.0" />
          <p className="pt-1 text-xs text-foreground/45">
            Save syncing and desktop integration are still being built. See the project
            plan for what is coming next.
          </p>
        </Section>
      </div>
    </div>
  );
}

function LibrarySection() {
  const queryClient = useQueryClient();
  const [path, setPath] = useState("");

  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const settings = useQuery({ queryKey: ["settings"], queryFn: () => backend.suggestLibraryRoot() });

  useEffect(() => {
    if (settings.data) setPath(settings.data);
  }, [settings.data]);

  async function browse() {
    setError(null);
    try {
      const chosen = await backend.pickFolder(path || undefined);
      if (chosen) setPath(chosen);
    } catch (e) {
      setError(messageOf(e));
    }
  }

  async function save() {
    setError(null);
    try {
      await backend.setLibraryRoot(path);
      setSaved(true);
      // The status card shows the library root, so keep it in step.
      await queryClient.invalidateQueries({ queryKey: ["status"] });
      setTimeout(() => setSaved(false), 2000);
    } catch (e) {
      setError(messageOf(e));
    }
  }

  return (
    <Section title="Library">
      <label className="text-xs text-foreground/55" htmlFor="settings-library-root">
        Games folder
      </label>
      <div className="flex gap-2">
        <input
          id="settings-library-root"
          value={path}
          onChange={(e) => setPath(e.target.value)}
          spellCheck={false}
          className="min-w-0 flex-1 rounded-lg border border-default-200 bg-content2 px-3 py-2 font-mono text-xs outline-none transition-colors focus:border-primary"
        />
        <button
          type="button"
          onClick={browse}
          className="shrink-0 rounded-lg border border-default-200 px-3 py-2 text-xs text-foreground/70 transition-colors hover:bg-default-100"
        >
          Browse…
        </button>
        <button
          type="button"
          onClick={save}
          disabled={!path.trim()}
          className="shrink-0 rounded-lg bg-primary px-3 py-2 text-xs font-medium text-white transition-colors hover:bg-primary-600 disabled:opacity-40"
        >
          {saved ? "Saved" : "Save"}
        </button>
      </div>
      {error && (
        <p role="alert" className="text-xs text-danger">
          {error}
        </p>
      )}
      <p className="text-[11px] text-foreground/45">
        Downloads go to <code className="text-foreground/60">Gameyfin/Downloads</code> and
        installs to <code className="text-foreground/60">Gameyfin/Installations</code>{" "}
        inside this folder.
      </p>

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
  const prefixes = useQuery({ queryKey: ["prefixes"], queryFn: () => backend.prefixInfo() });
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

      {prefixes.data && (
        <PathRow
          label="Compatibility prefixes"
          hint={
            prefixes.data.count === 0
              ? "None yet. One is created the first time a Windows game runs."
              : `${prefixes.data.count} prefix${prefixes.data.count === 1 ? "" : "es"}, ${formatBytes(prefixes.data.bytes)}. Rebuilt automatically if removed.`
          }
          path={prefixes.data.path}
          action={
            prefixes.data.count > 0
              ? {
                  label: "Delete all",
                  danger: true,
                  onClick: async () => {
                    await backend.clearPrefixes();
                    await prefixes.refetch();
                  },
                }
              : undefined
          }
        />
      )}

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
