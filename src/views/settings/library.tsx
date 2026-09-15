import { useState } from "react";

import { ConfirmDialog } from "@/components/ConfirmDialog";
import { useLibraryRoots } from "@/components/RootChooser";
import { Button, FormField, SwitchField, TextArea, TextInput } from "@/components/ui";
import { backend } from "@/lib/backend";
import { formatBytes } from "@/lib/format";
import { keys, useAppSettings, useInvalidate } from "@/lib/queries";
import { useAction } from "@/lib/useAction";
import { HINT } from "@/lib/ui";
import { SaveError, Section, useSettingSaver } from "./controls";

/** The games folders, and which one is the default. */
export function RootsSection() {
  const roots = useLibraryRoots();
  const invalidate = useInvalidate();
  const action = useAction();
  const [removing, setRemoving] = useState<string | null>(null);

  function run(work: () => Promise<void>) {
    return action.run(async () => {
      await work();
      await roots.refetch();
      // The wizard and the status card both read the primary folder.
      await invalidate(keys.status, keys.settings);
    });
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
                <Button
                  size="sm"
                  onClick={() => void run(() => backend.setDefaultLibraryRoot(root.path))}
                >
                  Make default
                </Button>
              )}
              <Button
                size="sm"
                icon="folder"
                onClick={() => void backend.openLibraryFolder("installations", root.path)}
              >
                Open folder
              </Button>
              {list.length > 1 && (
                <Button size="sm" variant="destructive" onClick={() => setRemoving(root.path)}>
                  Remove
                </Button>
              )}
            </div>
          </div>
        ))}
      </div>

      <div className="pt-1">
        <Button onClick={() => void add()}>Add a folder…</Button>
      </div>

      <SaveError error={action.error} />

      <p className={HINT}>
        Downloads go to <code className="text-foreground/60">Gameyfin/Downloads</code> and
        installs to <code className="text-foreground/60">Gameyfin/Installations</code>{" "}
        inside each of these. With more than one folder you are asked which to use when a
        download starts.
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

export function AutomationSection() {
  const settings = useAppSettings();
  const { save, error } = useSettingSaver();
  const [password, setPassword] = useState<string | null>(null);
  const [ignored, setIgnored] = useState<string | null>(null);

  const savedIgnored = (settings.data?.ignoredExecutables ?? []).join("\n");
  const autoInstall = settings.data?.autoInstall ?? false;

  // Saved when the field is left, not per keystroke. The draft is then dropped so the
  // field shows what the backend kept, with blank lines and stray spaces trimmed.
  function commit(patch: Parameters<typeof save>[0], clearDraft: () => void) {
    void save(patch).then(clearDraft);
  }

  return (
    <Section title="Automation">
      <SwitchField
        label="Extract automatically"
        hint="Unpack archives as soon as they download."
        checked={settings.data?.autoExtract ?? true}
        onChange={(next) => save({ autoExtract: next })}
      >
        <SwitchField
          label="Delete the archive after extracting"
          hint="Remove the archive once its files are unpacked."
          checked={settings.data?.deleteArchiveAfterExtract ?? true}
          onChange={(next) => save({ deleteArchiveAfterExtract: next })}
        />
        <FormField
          label="Archive password"
          htmlFor="extraction-password"
          hint="Tried on encrypted archives and kept in your settings file."
        >
          <TextInput
            id="extraction-password"
            type="password"
            mono
            value={password ?? ""}
            onChange={(e) => setPassword(e.target.value)}
            onBlur={() => {
              if (password !== null) commit({ extractionPassword: password }, () => setPassword(null));
            }}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
            spellCheck={false}
            placeholder={settings.data?.hasExtractionPassword ? "Saved, type to replace" : "None"}
          />
        </FormField>
      </SwitchField>

      <div className="pt-2">
        <SwitchField
          label="Install automatically when a download finishes"
          hint="Install games when their download finishes, running Inno Setup and NSIS installers silently."
          checked={autoInstall}
          onChange={(next) => save({ autoInstall: next })}
        >
          <SwitchField
            label="Delete the download after installing"
            hint="Remove the download once the game is installed."
            checked={settings.data?.deleteDownloadAfterInstall ?? false}
            onChange={(next) => save({ deleteDownloadAfterInstall: next })}
          />
          <SetupSwitches
            id="inno-setup-arguments"
            label="Inno Setup options"
            hint="Silent install switches for Inno Setup, empty for the default."
            saved={settings.data?.innoSetupArguments}
            disabled={!autoInstall}
            onCommit={(value, clearDraft) => commit({ innoSetupArguments: value }, clearDraft)}
          />
          <SetupSwitches
            id="nsis-arguments"
            label="NSIS options"
            hint="Silent install switches for NSIS, empty for the default."
            saved={settings.data?.nsisArguments}
            disabled={!autoInstall}
            onCommit={(value, clearDraft) => commit({ nsisArguments: value }, clearDraft)}
          />
          <FormField
            label="Never offer these executables"
            htmlFor="ignored-executables"
            hint="File names never picked as the game, one per line."
          >
            <TextArea
              id="ignored-executables"
              rows={6}
              mono
              value={ignored ?? savedIgnored}
              onChange={(e) => setIgnored(e.target.value)}
              onBlur={() => {
                if (ignored !== null && ignored !== savedIgnored) {
                  commit({ ignoredExecutables: ignored.split("\n") }, () => setIgnored(null));
                }
              }}
              spellCheck={false}
            />
          </FormField>
        </SwitchField>
      </div>

      <SaveError error={error} />
    </Section>
  );
}

function SetupSwitches({
  id,
  label,
  hint,
  saved,
  disabled,
  onCommit,
}: {
  id: string;
  label: string;
  hint: string;
  saved: string | undefined;
  disabled: boolean;
  onCommit: (value: string, clearDraft: () => void) => void;
}) {
  const [draft, setDraft] = useState<string | null>(null);
  return (
    <FormField label={label} htmlFor={id} hint={hint}>
      <TextInput
        id={id}
        mono
        disabled={disabled}
        value={draft ?? saved ?? ""}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => {
          if (draft !== null && draft !== saved) onCommit(draft, () => setDraft(null));
        }}
        onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
        spellCheck={false}
      />
    </FormField>
  );
}
