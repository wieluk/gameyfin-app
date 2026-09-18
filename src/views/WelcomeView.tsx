import { useEffect, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Icon } from "@/components/Icon";
import { Button, TextInput } from "@/components/ui";
import { backend } from "@/lib/backend";
import { useAction } from "@/lib/useAction";
import { useTauriEvent } from "@/lib/useTauriEvent";
import type { LoginProgress } from "@/bindings/LoginProgress";
import { Alert } from "@/components/Alert";
import { messageOf } from "@/lib/errors";
import { keys, useAppSettings } from "@/lib/queries";

/**
 * First-run setup: choose a server, sign in, choose where games live. Sign-in hands off
 * to a real browser window, so SSO, MFA and any OIDC provider work and the app never
 * touches the password.
 */

type Step = "server" | "signin" | "library";

export function WelcomeView({
  knownServer,
  onStarted,
  onComplete,
}: {
  /** Set when only the session is gone: the wizard resumes at sign in. */
  knownServer: string | null;
  /** The user has answered a step, so the wizard is theirs to finish. */
  onStarted: () => void;
  onComplete: () => void;
}) {
  const [resuming] = useState(Boolean(knownServer));
  const [step, setStep] = useState<Step>(knownServer ? "signin" : "server");
  const [serverUrl, setServerUrl] = useState(knownServer ?? "");
  const settings = useAppSettings();

  function signedIn() {
    onStarted();
    // A returning user already chose a folder; asking again would look like a reset.
    if (resuming && settings.data?.libraryRoot) onComplete();
    else setStep("library");
  }

  return (
    <div className="flex min-h-0 flex-1 items-center justify-center overflow-y-auto bg-background p-8">
      <div className="w-full max-w-md">
        <header className="mb-8 text-center">
          <div className="mb-4 flex justify-center">
            <Icon name="controller" className="h-12 w-12 text-primary" />
          </div>
          <h1 className="text-2xl font-semibold text-foreground">
            {resuming ? "Welcome back" : "Welcome to Gameyfin"}
          </h1>
          <p className="mt-1 text-sm text-foreground/55">
            {step === "server" && "Connect to your Gameyfin server."}
            {step === "signin" &&
              (resuming ? "Your session ended. Sign in again." : "Sign in to your account.")}
            {step === "library" && "Choose where games are stored, and name this PC."}
          </p>
        </header>

        <Steps current={step} />

        <div className="mt-6">
          {step === "server" && (
            <ServerStep
              initial={serverUrl}
              onDone={(url) => {
                onStarted();
                setServerUrl(url);
                setStep("signin");
              }}
            />
          )}
          {step === "signin" && (
            <SignInStep
              serverUrl={serverUrl}
              onDone={signedIn}
              onBack={() => setStep("server")}
            />
          )}
          {step === "library" && (
            <LibraryStep onDone={onComplete} onBack={() => setStep("signin")} />
          )}
        </div>
      </div>
    </div>
  );
}

/** Back navigation, present on every step after the first. */
function BackLink({ onClick, label = "Back" }: { onClick: () => void; label?: string }) {
  return (
    <Button size="sm" variant="ghost" className="self-start" onClick={onClick}>
      ← {label}
    </Button>
  );
}

const ORDER: Step[] = ["server", "signin", "library"];
const LABELS: Record<Step, string> = { server: "Server", signin: "Sign in", library: "Library" };

function Steps({ current }: { current: Step }) {
  const index = ORDER.indexOf(current);
  return (
    <ol className="flex items-center gap-2">
      {ORDER.map((step, i) => (
        <li key={step} className="flex flex-1 items-center gap-2">
          <div className="flex-1">
            <div
              className={`h-1 rounded-full transition-colors ${
                i <= index ? "bg-primary" : "bg-default-200"
              }`}
            />
            <span
              className={`mt-1.5 block text-[11px] ${
                i <= index ? "text-foreground/70" : "text-foreground/35"
              }`}
            >
              {LABELS[step]}
            </span>
          </div>
        </li>
      ))}
    </ol>
  );
}

function ServerStep({
  initial,
  onDone,
}: {
  initial: string;
  onDone: (url: string) => void;
}) {
  const [value, setValue] = useState(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!value.trim() || busy) return;

    setBusy(true);
    setError(null);
    try {
      const probe = await backend.probeServer(value);
      if (!probe.reachable) {
        setError(probe.message ?? "Could not reach that server.");
        return;
      }
      const saved = await backend.setServerUrl(probe.url);
      onDone(saved);
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={submit} className="flex flex-col gap-3">
      <label className="text-xs font-medium text-foreground/60" htmlFor="server-url">
        Server address
      </label>
      <TextInput
        id="server-url"
        size="lg"
        value={value}
        onChange={(e) => setValue(e.target.value)}
        placeholder="games.example.com"
        autoFocus
        spellCheck={false}
        autoCapitalize="none"
      />
      <p className="text-[11px] text-foreground/45">
        https:// is assumed if you leave the scheme out.
      </p>

      {error && <Alert>{error}</Alert>}

      <Button
        type="submit"
        variant="primary"
        size="lg"
        className="mt-1"
        disabled={busy || !value.trim()}
      >
        {busy ? "Checking…" : "Continue"}
      </Button>
    </form>
  );
}

/** How often the wizard polls the sign-in window. */
const POLL_INTERVAL_MS = 1000;

function SignInStep({
  serverUrl,
  onDone,
  onBack,
}: {
  serverUrl: string;
  onDone: () => void;
  onBack: () => void;
}) {
  const [waiting, setWaiting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [detail, setDetail] = useState<string | null>(null);
  const [host, setHost] = useState<string | null>(null);
  const [cleared, setCleared] = useState(false);
  const clearing = useAction();
  const queryClient = useQueryClient();
  const cancelled = useRef(false);

  useEffect(() => {
    cancelled.current = false;
    return () => {
      cancelled.current = true;
      void backend.cancelLogin();
    };
  }, []);

  // A login can cross server, identity provider and proxy; the origin is the only clue
  // to where a stalled one is.
  useTauriEvent<LoginProgress>("login-progress", (progress) => setHost(progress.host));

  // Poll the backend: it reports success only once the harvested cookies actually
  // authenticate, not merely once the provider has set some of its own.
  useEffect(() => {
    if (!waiting) return;
    // Chained rather than an interval: a poll that takes longer than the gap would
    // otherwise overlap itself, and a late answer could report on a cancelled login.
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;

    async function poll() {
      try {
        const result = await backend.pollLogin();
        if (stopped) return;
        if (result.signedIn) return onDone();
        setDetail(result.detail);
        if (!result.windowOpen) {
          // The user closed the window; waiting any longer would spin forever.
          setWaiting(false);
          setError("The login window was closed before sign in finished.");
          return;
        }
      } catch (e) {
        if (stopped) return;
        setWaiting(false);
        setError(messageOf(e));
        return;
      }
      timer = setTimeout(() => void poll(), POLL_INTERVAL_MS);
    }

    timer = setTimeout(() => void poll(), POLL_INTERVAL_MS);
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [waiting, onDone]);

  async function start(direct = false) {
    setError(null);
    setDetail(null);
    setCleared(false);
    try {
      await backend.beginLogin(direct);
      setWaiting(true);
    } catch (e) {
      setError(messageOf(e));
    }
  }

  // Clearing closes the sign-in window and waits for the webview to let go of its files,
  // which takes a moment on Windows.
  async function reset() {
    setError(null);
    setDetail(null);
    setHost(null);
    setWaiting(false);
    setCleared(false);
    const done = await clearing.run(async () => {
      await backend.resetLogin();
      // It signs out as well as clearing the window's data, so the status is now stale.
      await queryClient.invalidateQueries({ queryKey: keys.status });
      return true;
    });
    setCleared(Boolean(done));
  }

  return (
    <div className="flex flex-col gap-3">
      <BackLink onClick={onBack} label="Change server" />

      <div className="rounded-lg border border-default-200 bg-content2 px-3 py-2.5">
        <p className="text-[11px] text-foreground/45">Connecting to</p>
        <p className="truncate text-sm text-foreground">{serverUrl}</p>
      </div>

      {waiting ? (
        <>
          <div className="flex items-start gap-3 rounded-lg border border-primary/30 bg-primary/10 px-3 py-3">
            <span className="mt-1.5 h-2 w-2 shrink-0 animate-pulse rounded-full bg-primary" />
            <div className="min-w-0">
              <p className="text-sm text-foreground/75">
                Waiting for you to finish signing in…
              </p>
              {host && (
                <p className="mt-0.5 truncate font-mono text-[11px] text-foreground/45">
                  at {host}
                </p>
              )}
            </div>
          </div>

          {detail && (
            <p className="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning-600">
              {detail}
            </p>
          )}

          <p className="text-[11px] text-foreground/45">
            A window has opened for your server's login page. Single sign-on providers
            like Authentik are supported. Finish there and this continues on its own.
          </p>

          <Button
            size="lg"
            onClick={() => {
              setWaiting(false);
              void backend.cancelLogin();
            }}
          >
            Cancel
          </Button>
        </>
      ) : (
        <>
          {error && <Alert>{error}</Alert>}
          <Button variant="primary" size="lg" onClick={() => void start()}>
            Sign in
          </Button>
          <p className="text-[11px] text-foreground/45">
            Your server decides how you sign in: a single sign-on provider if it has one,
            otherwise a username and password.
          </p>

          <div className="mt-1 flex flex-col items-start gap-1.5 border-t border-default-200/60 pt-3">
            <Button size="sm" variant="ghost" onClick={() => void start(true)}>
              Use a username and password instead
            </Button>
            <Button
              size="sm"
              variant="ghost"
              className="text-left"
              onClick={reset}
              disabled={clearing.busy}
            >
              {clearing.busy
                ? "Clearing the saved login data…"
                : "Login window misbehaving? Sign out of it and clear its saved data."}
            </Button>
            {clearing.error && <Alert>{clearing.error}</Alert>}
            {cleared && (
              <p className="text-[11px] text-success-600">
                Cleared. The next sign in starts from scratch.
              </p>
            )}
          </div>
        </>
      )}
    </div>
  );
}

function LibraryStep({ onDone, onBack }: { onDone: () => void; onBack: () => void }) {
  const [path, setPath] = useState("");
  const [device, setDevice] = useState("");
  const [error, setError] = useState<string | null>(null);
  const action = useAction();

  useEffect(() => {
    // Only as a starting point: a suggestion that arrives late must not overwrite
    // whatever the user has started typing. A folder chosen before comes first.
    backend
      .getSettings()
      .then(async (settings) => settings.libraryRoot || (await backend.suggestLibraryRoot()))
      .then((initial) => setPath((current) => current || initial))
      .catch((e) => setError(messageOf(e)));
    // Prefilled with what the system calls itself, so the name is there to be changed
    // rather than being a box whose purpose is not obvious.
    backend
      .getSettings()
      .then(async (settings) => settings.deviceName || (await backend.detectedDeviceName()))
      .then((name) => setDevice((current) => current || (name ?? "")))
      .catch(() => undefined);
  }, []);

  async function finish() {
    setError(null);
    const done = await action.run(async () => {
      await backend.updateSettings({ libraryRoot: path, deviceName: device });
      return true;
    });
    if (done) onDone();
  }

  return (
    <div className="flex flex-col gap-3">
      <BackLink onClick={onBack} />
      <label className="text-xs font-medium text-foreground/60" htmlFor="library-root">
        Games folder
      </label>
      <div className="flex gap-2">
        <TextInput
          id="library-root"
          size="lg"
          mono
          value={path}
          onChange={(e) => setPath(e.target.value)}
          spellCheck={false}
        />
        <Button
          size="lg"
          onClick={async () => {
            try {
              const chosen = await backend.pickFolder(path || undefined);
              if (chosen) setPath(chosen);
            } catch (e) {
              setError(messageOf(e));
            }
          }}
        >
          Browse…
        </Button>
      </div>
      <p className="text-[11px] text-foreground/45">
        Downloads and installed games live here. Change it later in Settings.
      </p>

      <label className="pt-1 text-xs font-medium text-foreground/60" htmlFor="device-name">
        This device's name
      </label>
      <TextInput
        id="device-name"
        size="lg"
        value={device}
        onChange={(e) => setDevice(e.target.value)}
      />
      <p className="text-[11px] text-foreground/45">
        Shown beside saves synced from this PC, so you can tell your machines apart.
      </p>

      {error && <Alert>{error}</Alert>}

      <Button
        variant="primary"
        size="lg"
        className="mt-1"
        onClick={finish}
        disabled={!path.trim() || action.busy}
      >
        Finish
      </Button>
    </div>
  );
}
