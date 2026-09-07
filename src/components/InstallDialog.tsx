import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Icon } from "./Icon";
import { backend } from "@/lib/backend";
import { messageOf } from "@/lib/errors";
import type { LibraryEntry } from "@/types";
import { Alert } from "@/components/Alert";

/**
 * Choosing how to install a download.
 *
 * A Gameyfin library holds whatever its owner put there, an archive, a Windows installer,
 * a native Linux build, and those need different treatment. The app inspects the file and
 * offers only what applies, rather than assuming everything is a zip.
 *
 * For a setup program the app cannot know where files will land: the user drives the
 * wizard. So the suggested path is offered on the clipboard, and if they install somewhere
 * else they can point the app at it afterwards.
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
  /// The option currently being started, so the dialog can show which one and refuse a
  /// second click while it is under way.
  const [starting, setStarting] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  // Extracting is a mechanical step with one outcome; installing is where the choices
  // about setup programs and destinations belong. Showing both at once was confusing.
  const isExtractPhase = entry.state.kind === "downloaded";
  // Only meaningful for extraction: the archive is a duplicate of the unpacked files, and
  // for a large game that is a lot of disk to keep for no reason.
  const [deleteArchive, setDeleteArchive] = useState(true);

  // Keyed by state as well as game: what can be done with a download changes once it is
  // unpacked, and a plan cached from before extraction would keep offering "Extract".
  const plan = useQuery({
    queryKey: ["install-plan", gameId, entry.state.kind],
    queryFn: () => backend.installOptions(gameId),
    staleTime: 0,
    gcTime: 0,
  });

  async function start(method: string, interactive: boolean) {
    // Starting an install is not instant, a setup program has a prefix to prepare first,
    // and until now the dialog stayed fully interactive throughout, with every option
    // still clickable. A second click was swallowed by the backend's busy check rather
    // than starting anything twice, but from the user's side it simply looked like the
    // first click had not registered.
    if (starting) return;
    setStarting(method);
    setError(null);
    try {
      if (interactive && plan.data) {
        // Give the user the path before the wizard asks for it. Under a compatibility
        // layer that has to be the mapped drive letter, since a setup program cannot
        // navigate to a Linux path.
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

  // Detection is heuristic and can miss an oddly named installer, so the user can point
  // at one directly rather than being stuck with what was found.
  async function chooseSetup() {
    if (starting) return;
    setError(null);
    try {
      // Open where the files actually are, so the user is not navigating back.
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
              The app does not know how to install a {plan.data.payload} yet. You can open
              the folder and install it yourself, then use “I installed it myself”.
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
                  Frees the space it occupies. You would need to download the game again to
                  get it back.
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

          {/* Always offered, whatever was detected: the heuristics can miss an oddly
              named installer, and a game may already be installed elsewhere. Held back
              until the plan has loaded so both halves of the dialog appear together
              rather than this one arriving first, alone. */}
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
                  Pick the installer yourself if the right one was not found.
                </p>
              </button>
              <button
                type="button"
                onClick={locate}
                className="rounded-xl border border-default-200 p-3 text-left transition-colors hover:border-primary/50 hover:bg-primary/5"
              >
                <span className="text-sm font-medium text-foreground">
                  Point at an existing folder
                </span>
                <p className="mt-1 text-xs leading-relaxed text-foreground/55">
                  Already installed it? Show the app where the game lives.
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

