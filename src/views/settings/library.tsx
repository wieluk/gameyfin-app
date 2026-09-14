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

/** What a finished download should do next. */
export function DownloadSection() {
  const settings = useAppSettings();
  const { save, error } = useSettingSaver();

  return (
    <Section title="Downloads">
      <SwitchField
        label="Install automatically when a download finishes"
        hint="Unpacks the download and moves the game into your installations folder without asking. Downloads that contain a setup program still stop and wait for you."
        checked={settings.data?.autoInstall ?? false}
        onChange={(next) => save({ autoInstall: next })}
      />
      <SwitchField
        label="Delete the archive after extracting"
        hint="Frees the space the archive takes once its files are unpacked. Also the starting choice in the install dialog. Reinstalling means downloading again."
        checked={settings.data?.deleteArchiveAfterExtract ?? true}
        onChange={(next) => save({ deleteArchiveAfterExtract: next })}
      />
      <SwitchField
        label="Delete the download after installing"
        hint="Removes the archive and the unpacked files once a game installs successfully. Reinstalling means downloading again."
        checked={settings.data?.deleteDownloadAfterInstall ?? false}
        onChange={(next) => save({ deleteDownloadAfterInstall: next })}
      />
      <SaveError error={error} />
    </Section>
  );
}

/** The archive password and the executables never worth offering. */
export function ExtractionSection() {
  const settings = useAppSettings();
  const { save, error } = useSettingSaver();
  const [password, setPassword] = useState<string | null>(null);
  const [ignored, setIgnored] = useState<string | null>(null);

  const savedIgnored = (settings.data?.ignoredExecutables ?? []).join("\n");

  // Saved when the field is left, not per keystroke. The draft is then dropped so the
  // field shows what the backend kept, with blank lines and stray spaces trimmed.
  function commit(patch: Parameters<typeof save>[0], clearDraft: () => void) {
    void save(patch).then(clearDraft);
  }

  return (
    <Section title="Extraction">
      <FormField
        label="Archive password"
        htmlFor="extraction-password"
        hint="Tried automatically when an archive is encrypted. Stored in the app's settings file beside your session, readable only by you, and never shown again once saved. A convenience, not a secret store."
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

      <FormField
        className="pt-2"
        label="Never offer these executables"
        htmlFor="ignored-executables"
        hint="One per line, matched anywhere in the file name. Keeps redistributables and crash handlers from crowding out the real launcher."
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

      <SaveError error={error} />
    </Section>
  );
}
