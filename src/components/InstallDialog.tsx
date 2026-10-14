import { Children, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { SetupOptions } from "./GameOptions";
import { SetupPicker } from "./SetupList";
import { backend } from "@/lib/backend";
import { useFlash } from "@/lib/useFlash";
import { messageOf } from "@/lib/errors";
import { useAppSettings } from "@/lib/queries";
import { arranged, coversEverything, defaultSelection, runOrder, toggled } from "@/lib/setups";
import type { InstallPlan } from "@/bindings/InstallPlan";
import type { LibraryEntry } from "@/types";
import { Alert } from "@/components/Alert";
import type { IconName } from "@/components/Icon";
import { Button, IconButton, Switch, SwitchField } from "@/components/ui";
import { Modal } from "./Modal";
import { PANEL_BODY } from "@/lib/ui";

/**
 * Choosing how to install a download. For a setup program the suggested path goes to the
 * clipboard, because the wizard, not the app, picks the destination. The plan is loaded before
 * it opens, so it appears complete.
 */
export function InstallDialog({
  entry,
  plan,
  onClose,
}: {
  entry: LibraryEntry;
  plan: InstallPlan;
  onClose: () => void;
}) {

  const gameId = entry.game.id;
  const queryClient = useQueryClient();
  const [error, setError] = useState<string | null>(null);
  // The option being started, so a second click is refused meanwhile.
  const [starting, setStarting] = useState<string | null>(null);
  const [copied, flashCopied] = useFlash(1500);
  // Extract is mechanical; install is where the choices belong.
  const isExtractPhase = entry.state.kind === "downloaded" && entry.state.extractable;
  // Starts from the setting; null until toggled so a setting that loads late still applies.
  const settings = useAppSettings();
  const [deleteArchiveChoice, setDeleteArchive] = useState<boolean | null>(null);
  const deleteArchive = deleteArchiveChoice ?? settings.data?.deleteArchiveAfterExtract ?? true;
  const [deleteDownloadChoice, setDeleteDownload] = useState<boolean | null>(null);

  // Null until rearranged, so the plan's order stands.
  const [order, setOrder] = useState<string[] | null>(null);
  const setups = arranged(plan.setups, order);
  const recognised = setups.filter((s) => s.matched);
  // Every program in the download is listed, so only a recognised one makes this a setup install.
  const hasSetups = recognised.length > 0;
  // Null until touched, so the plan's recommendation applies once it loads.
  const [pickedSetups, setPickedSetups] = useState<string[] | null>(null);
  const selected = pickedSetups ?? defaultSelection(setups);
  // Off until asked for: a wizard shows what it installs and where.
  const [silent, setSilent] = useState(false);
  // Setups left unticked, such as DLC for later, keep the download.
  const deleteDownload =
    deleteDownloadChoice ??
    ((settings.data?.deleteDownloadAfterInstall ?? false) && coversEverything(setups, selected));

  async function start(method: string, interactive: boolean) {
    // Starting takes a moment while a prefix is prepared, so a second click is ignored.
    if (starting) return;
    setStarting(method);
    setError(null);
    try {
      if (interactive) {
        // The wizard gets the mapped drive letter, not a Linux path it cannot navigate to.
        await backend.copyToClipboard(
          plan.windowsInstallPath ?? plan.defaultInstallDir,
        );
      }
      await backend.install(gameId, method, deleteArchive, deleteDownload);
      await queryClient.invalidateQueries({ queryKey: ["entries"] });
      onClose();
    } catch (e) {
      setError(messageOf(e));
    } finally {
      setStarting(null);
    }
  }

  const toggle = (path: string, on: boolean) =>
    setPickedSetups(toggled(setups, selected, path, on));

  async function installSetups() {
    if (starting || selected.length === 0) return;
    setStarting("setups");
    setError(null);
    try {
      if (!silent) {
        await backend.copyToClipboard(
          plan.windowsInstallPath ?? plan.defaultInstallDir,
        );
      }
      await backend.installSetups(gameId, runOrder(setups, selected), silent, deleteDownload);
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
      const chosen = await backend.pickFile(plan.browseDir ?? plan.defaultInstallDir);
      if (!chosen) return;
      await backend.runSetupPath(gameId, chosen, deleteDownload);
      await queryClient.invalidateQueries({ queryKey: ["entries"] });
      onClose();
    } catch (e) {
      setError(messageOf(e));
    }
  }

  async function locate() {
    setError(null);
    try {
      const chosen = await backend.pickFolder(plan.defaultInstallDir);
      if (!chosen) return;
      await backend.locateInstall(gameId, chosen);
      await queryClient.invalidateQueries({ queryKey: ["entries"] });
      onClose();
    } catch (e) {
      setError(messageOf(e));
    }
  }

  /** Flags for a setup wizard and the path to paste into it, for the card that starts one. */
  // Called, not rendered as a component: one defined in here would remount on every render.
  function wizardSettings() {
    return (
      <>
        {/* Here rather than in the installed game's options: a silent-install flag only
            does anything if it is set before the wizard runs. */}
        <SetupOptions
          gameId={gameId}
          hint="Passed to the setup program, and kept for this game. A silent-install flag is what lets it install without asking anything."
        />
        {/* A silent install is told where to go, so there is nothing to type. */}
        {plan.needsInstallPath && !(hasSetups && silent) && (
          <div>
            <p className="mb-1 text-xs text-foreground/55">Type this into the installer</p>
            <div className="flex items-start gap-2">
              <code className="min-w-0 flex-1 truncate rounded-lg border border-default-200 bg-content2 px-3 py-1.5 font-mono text-xs text-foreground/80">
                {plan.windowsInstallPath ?? plan.defaultInstallDir}
              </code>
              <Button
                onClick={async () => {
                  await backend.copyToClipboard(plan.windowsInstallPath ?? plan.defaultInstallDir);
                  flashCopied();
                }}
              >
                {copied ? "Copied" : "Copy"}
              </Button>
            </div>
          </div>
        )}
      </>
    );
  }

  const summary =
    recognised.length > 0
      ? `Unpacked, with ${recognised.length} setup program${recognised.length === 1 ? "" : "s"}.`
      : entry.state.kind === "extracted"
        ? "Unpacked, with no setup program."
        : `This download is a ${plan.payload}.`;

  return (
    <Modal
      label={`Install ${entry.game.title}`}
      size="2xl"
      layer={50}
      pad={4}
      fitted
      onDismiss={onClose}
    >
      <header className="flex items-start justify-between gap-4 border-b border-default-200/60 px-6 py-5">
        <div className="min-w-0">
          <h2 className="truncate text-base font-semibold text-foreground">
            {isExtractPhase ? "Extract" : "Install"} {entry.game.title}
          </h2>
          <p className="mt-1 text-xs text-foreground/50">{summary}</p>
        </div>
        <IconButton icon="close" label="Close" size="sm" onClick={onClose} />
      </header>

      <div className={PANEL_BODY}>
        {plan.options.length === 0 && !hasSetups && (
          <Alert>
            This app cannot install a {plan.payload} yet. Install it yourself, then
            point the app at the folder below.
          </Alert>
        )}

        {/* One card per way of installing, each with its own button, so what belongs to which
            choice is never in doubt. The first is the one to pick when unsure. */}
        <div className="flex flex-col">
          {hasSetups && (
            <MethodCard
              title="Run the setup programs"
              description="The game's own installers, in the order they run. Drag to change it."
              recommended={plan.options.length > 0}
              primary
              action={selected.length > 1 ? `Install ${selected.length} setups` : "Install"}
              actionIcon="installed"
              busy={starting === "setups"}
              disabled={selected.length === 0 || starting !== null}
              onAction={() => void installSetups()}
            >
              <SetupPicker
                setups={setups}
                selected={selected}
                onToggle={toggle}
                onOrder={setOrder}
                disabled={starting !== null}
              />
              <Switch
                label="Install silently"
                checked={silent}
                onChange={setSilent}
                disabled={!setups.some((s) => s.silent) || starting !== null}
              />
              {wizardSettings()}
            </MethodCard>
          )}

          {plan.options.map((option, index) => (
            <div key={option.key}>
              {(hasSetups || index > 0) && <Or />}
              <MethodCard
                title={option.label}
                description={option.description}
                recommended={!hasSetups && index === 0 && plan.options.length > 1}
                primary={!hasSetups && index === 0}
                action={ACTION_LABELS[option.key] ?? "Start"}
                busy={starting === option.key}
                disabled={Boolean(option.blockedBy) || starting !== null}
                onAction={() => void start(option.key, option.interactive)}
              >
                {option.blockedBy && (
                  <p className="rounded-lg bg-warning/10 px-2 py-1 text-[0.6875rem] text-warning-600">
                    {option.blockedBy}
                  </p>
                )}
                {option.key === "extract" && (
                  <SwitchField
                    label="Delete the archive after extracting"
                    hint="Frees disk space. The only way back is downloading again."
                    checked={deleteArchive}
                    onChange={setDeleteArchive}
                  />
                )}
                {option.interactive && !hasSetups && wizardSettings()}
              </MethodCard>
            </div>
          ))}
        </div>

        {/* Shared by every card above, so it sits outside them. */}
        {(hasSetups || plan.options.some((o) => o.key !== "extract")) && (
          <div className="mt-4 rounded-lg border border-default-200 px-3 py-2">
            <SwitchField
              label="Delete the download after installing"
              hint={
                coversEverything(setups, selected)
                  ? "Applies to whichever way you install. Removes the download once the game is in place."
                  : "Off by default, since some setup programs are left for later."
              }
              checked={deleteDownload}
              onChange={setDeleteDownload}
            />
          </div>
        )}

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

        {error && <Alert className="mt-3">{error}</Alert>}
      </div>
    </Modal>
  );
}

/** What each install method's button says. */
const ACTION_LABELS: Record<string, string> = {
  extract: "Extract",
  move: "Move",
  "copy-executable": "Move",
  "run-installer": "Run installer",
  "run-installer-proton": "Run installer",
};

/** One way of installing: what it does, what it needs, and the button that starts it. */
function MethodCard({
  title,
  description,
  recommended,
  primary = recommended,
  action,
  actionIcon,
  busy,
  disabled,
  onAction,
  children,
}: {
  title: string;
  description: string;
  recommended: boolean;
  /** The first way to install gets the main button, highlighted or not. */
  primary?: boolean;
  action: string;
  actionIcon?: IconName;
  busy: boolean;
  disabled: boolean;
  onAction: () => void;
  children?: React.ReactNode;
}) {
  return (
    <section
      className={`rounded-xl border p-4 ${
        recommended ? "border-primary/40 bg-primary/5" : "border-default-200"
      }`}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="flex items-center gap-2 text-[0.9375rem] font-medium text-foreground">
            {title}
            {recommended && (
              <span className="rounded bg-primary/15 px-1.5 py-0.5 text-[0.625rem] font-medium text-primary">
                Recommended
              </span>
            )}
          </h3>
          <p className="mt-1 text-[0.8125rem] leading-relaxed text-foreground/55">{description}</p>
        </div>
        <Button
          variant={primary ? "primary" : "secondary"}
          icon={actionIcon}
          className="shrink-0"
          disabled={disabled}
          onClick={onAction}
        >
          {action}
          {busy && (
            <span
              aria-hidden
              className={`h-3 w-3 animate-spin rounded-full border-2 ${
                primary ? "border-white/40 border-t-white" : "border-primary/30 border-t-primary"
              }`}
            />
          )}
        </Button>
      </div>
      {/* Children can all be `false`, which would leave an empty gap under the text. */}
      {Children.toArray(children).length > 0 && (
        <div className="mt-3 flex flex-col gap-3">{children}</div>
      )}
    </section>
  );
}

/** Between two ways of installing, which are alternatives rather than steps. */
function Or() {
  return (
    <div className="flex items-center gap-3 py-2 text-[0.6875rem] uppercase tracking-wide text-foreground/40">
      <span className="h-px flex-1 bg-default-200" />
      or
      <span className="h-px flex-1 bg-default-200" />
    </div>
  );
}
