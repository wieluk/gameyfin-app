import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Icon } from "./Icon";
import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import type { LibraryEntry } from "@/types";
import { Alert } from "@/components/Alert";

/**
 * Choosing how to install a download. Only applicable options are offered, since a
 * library holds archives, Windows installers and native builds. For a setup program the
 * suggested path goes to the clipboard, because the wizard, not the app, picks the
 * destination.
 */
export function InstallDialog({
  entry,
  onClose,
}: {
  entry: LibraryEntry;
  onClose: () => void;
}) {
  const gameId = entry.game.id;
  const queryClient = useQueryClient();
  const [error, setError] = useState<string | null>(null);
  // The option being started, so a second click is refused meanwhile.
  const [starting, setStarting] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  // Extract is mechanical; install is where the choices belong. Showing both at once was confusing.
  const isExtractPhase = entry.state.kind === "downloaded";
  // The archive duplicates the unpacked files, so for a large game it is a lot of disk for nothing.
  const [deleteArchive, setDeleteArchive] = useState(true);

  // Keyed by state too: a plan cached from before extraction would keep offering "Extract".
  const plan = useQuery({
    queryKey: ["install-plan", gameId, entry.state.kind],
    queryFn: () => backend.installOptions(gameId),
    staleTime: 0,
    gcTime: 0,
  });

  async function start(method: string, interactive: boolean) {
    // Starting takes a moment (a prefix may be prepared first); a second click used to
    // look like the first never registered.
    if (starting) return;
    setStarting(method);
    setError(null);
    try {
      if (interactive && plan.data) {
        // The wizard gets the mapped drive letter, not a Linux path it cannot navigate to.
        await backend.copyToClipboard(
          plan.data.windowsInstallPath ?? plan.data.defaultInstallDir,
        );
      }
      await backend.install(gameId, method, deleteArchive);
      await queryClient.invalidateQueries({ queryKey: ["entries"] });
      onClose();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setStarting(null);
    }
  }

  // Detection can miss an oddly named installer, so the user can point at one directly.
  async function chooseSetup() {
    if (starting) return;
    setError(null);
    try {
      const chosen = await backend.pickFile(plan.data?.browseDir ?? plan.data?.defaultInstallDir);
      if (!chosen) return;
      await backend.runSetupPath(gameId, chosen);
      await queryClient.invalidateQueries({ queryKey: ["entries"] });
      onClose();
    } catch (e) {
      setError(messageOf(e));
    }
  }

  async function locate() {
    setError(null);
    try {
      const chosen = await backend.pickFolder(plan.data?.defaultInstallDir);
      if (!chosen) return;
      await backend.locateInstall(gameId, chosen);
      await queryClient.invalidateQueries({ queryKey: ["entries"] });
      onClose();
    } catch (e) {
      setError(messageOf(e));
    }
  }

  return (
    <div
      data-nav-scope
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm"
      role="dialog"
      aria-modal="true"
      aria-label={`Install ${entry.game.title}`}
      onClick={onClose}
    >
      <div
        className="flex max-h-full w-full max-w-2xl flex-col overflow-hidden rounded-2xl border border-default-200 bg-content1 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="flex items-start justify-between gap-4 border-b border-default-200/60 px-6 py-5">
          <div className="min-w-0">
            <h2 className="truncate text-base font-semibold text-foreground">
              {isExtractPhase ? "Extract" : "Install"} {entry.game.title}
            </h2>
            {plan.data && (
              <p className="mt-1 text-xs text-foreground/50">
                This download is a {plan.data.payload}.
              </p>
            )}
          </div>
          <button
            type="button"
            aria-label="Close"
            onClick={onClose}
            className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-foreground/50 transition-colors hover:bg-default-100 hover:text-foreground"
          >
            <Icon name="close" className="h-3.5 w-3.5" />
          </button>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
          {plan.isLoading && <p className="text-xs text-foreground/50">Inspecting the download…</p>}

          {plan.isError && (
            <Alert>{messageOf(plan.error)}</Alert>
          )}

          {plan.data && plan.data.options.length === 0 && (
            <Alert>
              This app cannot install a {plan.data.payload} yet. Install it yourself, then
              point the app at the folder below.
            </Alert>
          )}

          {plan.data?.options.some((o) => o.key === "extract") && (
            <label className="mb-3 flex cursor-pointer items-start gap-2 rounded-lg border border-default-200 px-3 py-2">
              <input
                type="checkbox"
                checked={deleteArchive}
                onChange={(e) => setDeleteArchive(e.target.checked)}
                className="mt-0.5"
              />
              <span className="text-[13px] text-foreground/75">
                Delete the archive after extracting
                <span className="mt-1 block text-xs text-foreground/45">
                  Frees disk space. The only way back is downloading again.
                </span>
              </span>
            </label>
          )}

          <div className="flex flex-col gap-2">
            {plan.data?.options.map((option) => (
              <button
                key={option.key}
                type="button"
                disabled={Boolean(option.blockedBy) || starting !== null}
                onClick={() => void start(option.key, option.interactive)}
                className="rounded-xl border border-default-200 p-4 text-left transition-colors hover:border-primary/50 hover:bg-primary/5 disabled:cursor-not-allowed disabled:opacity-60 disabled:hover:border-default-200 disabled:hover:bg-transparent"
              >
                <span className="flex items-center gap-2 text-[15px] font-medium text-foreground">
                  {option.label}
                  {starting === option.key && (
                    <span
                      aria-hidden
                      className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-primary/30 border-t-primary"
                    />
                  )}
                </span>
                <p className="mt-1 text-[13px] leading-relaxed text-foreground/55">
                  {option.description}
                </p>
                {option.blockedBy && (
                  <p className="mt-1.5 rounded-lg bg-warning/10 px-2 py-1 text-[11px] text-warning-600">
                    {option.blockedBy}
                  </p>
                )}
              </button>
            ))}
          </div>

          {plan.data?.needsInstallPath && (
            <div className="mt-4 border-t border-default-200/60 pt-3">
              <p className="mb-1 text-xs text-foreground/50">
                Type this into the installer
              </p>
              <div className="flex gap-2">
                <code className="min-w-0 flex-1 truncate rounded-lg border border-default-200 bg-content2 px-3 py-2 font-mono text-[13px] text-foreground/80">
                  {plan.data.windowsInstallPath ?? plan.data.defaultInstallDir}
                </code>
                <button
                  type="button"
                  onClick={async () => {
                    await backend.copyToClipboard(
                      plan.data.windowsInstallPath ?? plan.data.defaultInstallDir,
                    );
                    setCopied(true);
                    setTimeout(() => setCopied(false), 1500);
                  }}
                  className="shrink-0 rounded-lg border border-default-200 px-2.5 py-1.5 text-[11px] text-foreground/70 transition-colors hover:bg-default-100"
                >
                  {copied ? "Copied" : "Copy"}
                </button>
              </div>
            </div>
          )}

          {/* Held until the plan loads so both halves of the dialog appear together. */}
          {plan.data && (
          <div className="mt-4 flex flex-col gap-2 border-t border-default-200/60 pt-4">
            <p className="text-xs font-semibold uppercase tracking-wide text-foreground/40">
              Do it manually
            </p>
            <div className="grid gap-2 sm:grid-cols-2">
              <button
                type="button"
                onClick={chooseSetup}
                className="rounded-xl border border-default-200 p-3 text-left transition-colors hover:border-primary/50 hover:bg-primary/5"
              >
                <span className="text-sm font-medium text-foreground">
                  Choose a setup program
                </span>
                <p className="mt-1 text-xs leading-relaxed text-foreground/55">
                  For when the right installer was not found.
                </p>
              </button>
              <button
                type="button"
                onClick={locate}
                className="rounded-xl border border-default-200 p-3 text-left transition-colors hover:border-primary/50 hover:bg-primary/5"
              >
                <span className="text-sm font-medium text-foreground">
                  Installed it yourself
                </span>
                <p className="mt-1 text-xs leading-relaxed text-foreground/55">
                  Show the app where the game lives.
                </p>
              </button>
            </div>
          </div>
          )}

          {error && <div className="mt-3"><Alert>{error}</Alert></div>}
        </div>
      </div>
    </div>
  );
}

