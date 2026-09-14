import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { SetupOptions } from "./GameOptions";
import { backend } from "@/lib/backend";
import { useFlash } from "@/lib/useFlash";
import { messageOf } from "@/lib/errors";
import { useAppSettings } from "@/lib/queries";
import type { LibraryEntry } from "@/types";
import { Alert } from "@/components/Alert";
import { Button, IconButton, SwitchField } from "@/components/ui";
import { Modal } from "./Modal";
import { PANEL_BODY } from "@/lib/ui";

/**
 * Choosing how to install a download. For a setup program the suggested path goes to the
 * clipboard, because the wizard, not the app, picks the destination.
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
  const [copied, flashCopied] = useFlash(1500);
  // Extract is mechanical; install is where the choices belong.
  const isExtractPhase = entry.state.kind === "downloaded";
  // Starts from the setting; null until toggled so a setting that loads late still applies.
  const settings = useAppSettings();
  const [deleteArchiveChoice, setDeleteArchive] = useState<boolean | null>(null);
  const deleteArchive = deleteArchiveChoice ?? settings.data?.deleteArchiveAfterExtract ?? true;

  // Keyed by state too: a plan cached from before extraction would keep offering "Extract".
  const plan = useQuery({
    queryKey: ["install-plan", gameId, entry.state.kind],
    queryFn: () => backend.installOptions(gameId),
    staleTime: 0,
    gcTime: 0,
  });

  async function start(method: string, interactive: boolean) {
    // Starting takes a moment while a prefix is prepared, so a second click is ignored.
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
    <Modal label={`Install ${entry.game.title}`} size="2xl" layer={50} pad={4} onDismiss={onClose}>
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
        <IconButton icon="close" label="Close" size="sm" onClick={onClose} />
      </header>

      <div className={PANEL_BODY}>
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
          <div className="mb-3 rounded-lg border border-default-200 px-3 py-2">
            <SwitchField
              label="Delete the archive after extracting"
              hint="Frees disk space. The only way back is downloading again."
              checked={deleteArchive}
              onChange={setDeleteArchive}
            />
          </div>
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

        {/* Here rather than in the installed game's options: a silent-install flag only
            does anything if it is set before the wizard runs. */}
        {plan.data?.options.some((option) => option.interactive) && (
          <div className="mt-4 border-t border-default-200/60 pt-3">
            <SetupOptions
              gameId={gameId}
              hint="Passed to the setup program, and kept for this game. A silent-install flag is what lets it install without asking anything."
            />
          </div>
        )}

        {plan.data?.needsInstallPath && (
          <div className="mt-4 border-t border-default-200/60 pt-3">
            <p className="mb-1 text-xs text-foreground/55">
              Type this into the installer
            </p>
            <div className="flex items-start gap-2">
              <code className="min-w-0 flex-1 truncate rounded-lg border border-default-200 bg-content2 px-3 py-1.5 font-mono text-xs text-foreground/80">
                {plan.data.windowsInstallPath ?? plan.data.defaultInstallDir}
              </code>
              <Button
                onClick={async () => {
                  await backend.copyToClipboard(
                    plan.data.windowsInstallPath ?? plan.data.defaultInstallDir,
                  );
                  flashCopied();
                }}
              >
                {copied ? "Copied" : "Copy"}
              </Button>
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

        {error && <Alert className="mt-3">{error}</Alert>}
      </div>
    </Modal>
  );
}
