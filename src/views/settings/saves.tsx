import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Button, FormField, Radio, Select, SwitchField, TextField, TextInput } from "@/components/ui";
import { backend } from "@/lib/backend";
import { formatBytes, formatRelative } from "@/lib/format";
import { keys, useAppSettings, useInvalidate, useSaveToolStatus } from "@/lib/queries";
import { useAction } from "@/lib/useAction";
import { useTauriEvent } from "@/lib/useTauriEvent";
import { HINT } from "@/lib/ui";
import type { MigrationSummary } from "@/bindings/MigrationSummary";
import type { SaveBackend } from "@/types";
import { VersionSection, type VersionTool } from "./VersionSection";
import { Row, SaveError, Section, useSettingSaver } from "./controls";

/** The three places saves can live, as the user sees them named. */
export const BACKEND_LABELS: Record<SaveBackend, string> = {
  server: "Gameyfin server",
  folder: "A folder",
  webdav: "WebDAV",
};

export const BACKENDS = Object.keys(BACKEND_LABELS) as SaveBackend[];

/** Save sync, on from the start against the Gameyfin server. */
export function SavesSection() {
  const settings = useAppSettings();
  const { save, error } = useSettingSaver();
  const test = useAction();
  const [result, setResult] = useState<string | null>(null);

  const data = settings.data;
  const active: SaveBackend = data?.saveBackend ?? "server";
  const enabled = data?.saveSyncEnabled ?? true;

  // A patch, never the whole set: two controls changed in quick succession would write
  // each other's stale value back.
  function update(patch: Parameters<typeof save>[0]) {
    setResult(null);
    return save(patch);
  }

  return (
    <>
      <SwitchField
        label="Sync my saves"
        hint="Backs up your saves after you play so another PC can pick them up."
        checked={enabled}
        onChange={(next) => update({ saveSyncEnabled: next })}
      />
      {/* Indented and greyed out together, because neither does anything on its own. */}
      <div className="ml-3 flex flex-col gap-2 border-l border-default-200/60 pl-3">
        <SwitchField
          label="Restore before a game starts"
          hint="Fetches a newer save from another PC before launching, so you carry on where you left off."
          checked={data?.syncSavesOnLaunch ?? true}
          disabled={!enabled}
          onChange={(next) => update({ syncSavesOnLaunch: next })}
        />
        <SwitchField
          label="Back up after a game closes"
          hint="Uploads your save when you finish playing. Nothing is uploaded if it has not changed."
          checked={data?.syncSavesOnExit ?? true}
          disabled={!enabled}
          onChange={(next) => update({ syncSavesOnExit: next })}
        />
      </div>

      <div role="radiogroup" aria-label="Where saves are kept" className="mt-4 flex flex-col gap-2">
        <p className="text-xs text-foreground/55">Where saves are kept</p>
        {(
          [
            ["server", "Needs a server with save sync turned on."],
            ["folder", "Any folder something else syncs: Syncthing, rclone, a NextCloud or Dropbox folder."],
            ["webdav", "A NextCloud, ownCloud or other WebDAV share, without mounting it first."],
          ] as Array<[SaveBackend, string]>
        ).map(([id, hint]) => (
          <label key={id} className="flex cursor-pointer items-start gap-2">
            <Radio
              name="save-backend"
              className="mt-0.5"
              checked={active === id}
              onChange={() => update({ saveBackend: id })}
            />
            <span>
              <span className="block text-xs text-foreground/80">{BACKEND_LABELS[id]}</span>
              <span className="mt-0.5 block text-[11px] leading-relaxed text-foreground/45">
                {hint}
              </span>
            </span>
          </label>
        ))}
      </div>

      {active === "folder" && <FolderFields onChanged={() => setResult(null)} />}
      {active === "webdav" && <WebDavFields onChanged={() => setResult(null)} />}

      {/* Directly under the location, since that is what it checks. */}
      <div className="mt-3 flex items-center gap-3">
        <Button
          onClick={() => void test.run(async () => setResult(await backend.testSaveStore()))}
          disabled={test.busy}
        >
          {test.busy ? "Checking…" : "Test connection"}
        </Button>
        {result && <span className="text-xs text-success-600">{result}</span>}
        {test.error && <span className="text-xs text-danger">{test.error}</span>}
      </div>
      <SaveError error={error} />

      {active !== "server" && (
        <div className="mt-3">
          <TextField
            label="Versions to keep per game"
            type="number"
            value={String(data?.saveMaxVersions ?? 10)}
            onCommit={(value) => update({ saveMaxVersions: Math.max(1, Number(value) || 10) })}
          />
        </div>
      )}
    </>
  );
}

/** The folder a folder store uses. Shared by the location picker and copying saves across. */
function FolderFields({ onChanged }: { onChanged?: () => void }) {
  const settings = useAppSettings();
  const { save, error } = useSettingSaver();
  const folder = settings.data?.saveFolder ?? "";

  async function browse() {
    const chosen = await backend.pickFolder(folder || undefined);
    if (!chosen) return;
    onChanged?.();
    await save({ saveFolder: chosen });
  }

  return (
    <div className="mt-3 flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <TextInput mono placeholder="No folder chosen" readOnly value={folder} />
        <Button onClick={() => void browse()}>Browse…</Button>
      </div>
      {/* The sandbox only reaches the home directory and removable media, the same
          limit the games folder already has. */}
      <p className={HINT}>In the Flatpak build the folder has to be inside your home directory.</p>
      <SaveError error={error} />
    </div>
  );
}

/** A WebDAV share's address and sign-in. Shared by the location picker and copying saves. */
function WebDavFields({ onChanged }: { onChanged?: () => void }) {
  const settings = useAppSettings();
  const { save, error } = useSettingSaver();
  const data = settings.data;

  function update(patch: Parameters<typeof save>[0]) {
    onChanged?.();
    return save(patch);
  }

  return (
    <div className="mt-3 flex flex-col gap-2">
      <TextField
        label="Address"
        placeholder="https://cloud.example.com/remote.php/dav/files/me/saves"
        value={data?.webdavUrl ?? ""}
        onCommit={(value) => update({ webdavUrl: value })}
      />
      <TextField
        label="Username"
        value={data?.webdavUsername ?? ""}
        onCommit={(value) => update({ webdavUsername: value })}
      />
      <TextField
        label="Password"
        type="password"
        value=""
        placeholder={data?.hasWebdavPassword ? "Saved, type to replace" : ""}
        onCommit={(value) => update({ webdavPassword: value })}
      />
      <p className="text-[11px] text-warning-600">
        This password is stored unencrypted in this PC's settings file, and never shown
        again once saved. Use an app password rather than your account password if your
        server offers one.
      </p>
      <SaveError error={error} />
    </div>
  );
}

/** Each place as it reads in a sentence. */
const PLACE: Record<SaveBackend, string> = {
  server: "the Gameyfin server",
  folder: "the save folder",
  webdav: "the WebDAV share",
};

/** Whether a place has what it needs to be reached. The server only needs a sign-in. */
function isSetUp(
  place: SaveBackend,
  data?: { saveFolder?: string | null; webdavUrl?: string | null },
): boolean {
  if (place === "folder") return Boolean(data?.saveFolder?.trim());
  if (place === "webdav") return Boolean(data?.webdavUrl?.trim());
  return true;
}

/** Copying saves into the place they are kept now, from one of the other places. */
export function MigrationSection() {
  const settings = useAppSettings();
  const data = settings.data;
  const active: SaveBackend = data?.saveBackend ?? "server";
  const sources = BACKENDS.filter((place) => place !== active);

  const [from, setFrom] = useState<SaveBackend>(sources[0]);
  // The place just switched away from, which is where a copy is usually wanted from.
  const [switchedFrom, setSwitchedFrom] = useState<SaveBackend | null>(null);
  const [allVersions, setAllVersions] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [summary, setSummary] = useState<MigrationSummary | null>(null);
  const migration = useAction();
  const busy = migration.busy;
  const seen = useRef<SaveBackend | null>(null);

  // Only a change after the settings loaded counts as switching.
  useEffect(() => {
    if (!data) return;
    if (seen.current !== null && seen.current !== active) {
      setFrom(seen.current);
      setSwitchedFrom(seen.current);
      setSummary(null);
    } else if (from === active) {
      setFrom(sources[0]);
    }
    seen.current = active;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, data]);

  // Subscribed only while a migration is running.
  useTauriEvent<{ done: number; total: number }>("save-migration-progress", setProgress, busy);

  async function run() {
    setSummary(null);
    setProgress(null);
    const result = await migration.run(() => backend.migrateSaves(from, allVersions));
    setProgress(null);
    if (result) {
      setSummary(result);
      setSwitchedFrom(null);
    }
  }

  const ready = isSetUp(active, data) && isSetUp(from, data);

  return (
    <Section title="Copy saves here">
      <p className={HINT}>
        Copies into {PLACE[active]}, where saves are kept now. Nothing is removed from the
        place you copy from, and a second run only copies what is missing.
      </p>

      {switchedFrom && (
        <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 rounded-lg border border-primary/30 bg-primary/5 px-3 py-2 text-xs">
          <span className="min-w-0 flex-1">
            You switched from {PLACE[switchedFrom]}. Copy your saves from there, so they are
            here too?
          </span>
          <Button size="sm" variant="ghost" onClick={() => setSwitchedFrom(null)}>
            Not now
          </Button>
        </div>
      )}

      <FormField className="pt-2" label="Copy from" htmlFor="migrate-from">
        <Select
          id="migrate-from"
          value={from}
          disabled={busy}
          onChange={(e) => {
            setFrom(e.target.value as SaveBackend);
            setSummary(null);
          }}
        >
          {sources.map((place) => (
            <option key={place} value={place}>
              {BACKEND_LABELS[place]}
            </option>
          ))}
        </Select>
      </FormField>
      {/* The source is set up right here, without making it the place saves are kept. */}
      {from === "folder" && <FolderFields />}
      {from === "webdav" && <WebDavFields />}

      <div className="pt-2">
        <SwitchField
          label="Copy every version"
          hint="Off by default, which copies only each game's newest save."
          checked={allVersions}
          onChange={setAllVersions}
        />
      </div>

      <div className="flex flex-wrap items-center gap-3 pt-1">
        <Button variant="primary" disabled={busy || !ready} onClick={() => void run()}>
          {busy ? "Copying…" : "Copy saves"}
        </Button>
        {busy && progress && (
          <span className="text-xs text-foreground/55">
            {progress.done} of {progress.total} games
          </span>
        )}
      </div>

      {!isSetUp(active, data) && (
        <p className={HINT}>Set up where saves are kept, above, before copying into it.</p>
      )}

      <SaveError error={migration.error} />

      {summary && summary.games === 0 && (
        <p className="pt-1 text-xs text-foreground/60">Nothing to copy: no saves were found there.</p>
      )}
      {summary && summary.games > 0 && (
        <div className="flex flex-col gap-1 pt-1">
          <p className={`text-xs ${summary.failed > 0 ? "text-warning-600" : "text-success-600"}`}>
            {summary.copied} copied from {summary.games} game
            {summary.games === 1 ? "" : "s"}
            {summary.copied > 0 ? ` (${formatBytes(summary.bytes)})` : ""}
            {summary.skipped > 0 ? `, ${summary.skipped} already here` : ""}
            {summary.failed > 0 ? `, ${summary.failed} failed` : ""}
          </p>
          {summary.problems.map((problem) => (
            <p key={problem} className="text-[11px] leading-relaxed text-danger">
              {problem}
            </p>
          ))}
        </div>
      )}
    </Section>
  );
}

/** Ludusavi, which finds and packs the save files. One ships with the app. */
export function SaveToolSection() {
  const settings = useAppSettings();
  const { save, error } = useSettingSaver();
  const invalidate = useInvalidate();
  const manifest = useAction();

  const status = useSaveToolStatus();
  const helper = status.data;

  function updateManifest() {
    return manifest.run(async () => {
      await backend.updateSaveManifest();
      await status.refetch();
      // A game that was unrecognised may be in the new database, so the verdicts are stale.
      await invalidate(keys.saveOverviewAll);
    });
  }

  const tool: VersionTool = {
    title: "Ludusavi",
    id: "save-tool",
    progressEvent: "save-tool-progress",
    loading: status.isLoading,
    info: helper && {
      version: helper.installed?.version ?? helper.bundled,
      builtIn: !helper.installed && helper.bundled !== null,
      latest: helper.latest?.version ?? null,
      downloadBytes: helper.latest?.sizeBytes ?? null,
      available: helper.available,
      updatable: Boolean(
        helper.latest &&
          (helper.installed?.version ?? helper.bundled) !== helper.latest.version,
      ),
    },
    install: async (version) => void (await backend.installSaveTool(version)),
    remove: () => backend.removeSaveTool(),
    after: async () => {
      await status.refetch();
    },
  };

  return (
    <VersionSection tool={tool}>
      <p className={HINT}>
        Gameyfin uses Ludusavi to find where each game keeps its saves. A copy ships with
        the app, and removing a downloaded one falls back to it. Your backups are not
        touched either way.
      </p>

      <div className="mt-2 flex flex-col gap-2 border-t border-default-200 pt-3">
        <Row
          label="Game database"
          value={
            helper?.manifest
              ? `${formatRelative(helper.manifest.updatedAt)}, ${formatBytes(helper.manifest.bytes)}`
              : "Not downloaded yet"
          }
        />
        <div className="flex flex-wrap items-center gap-3">
          <Button disabled={manifest.busy} onClick={() => void updateManifest()}>
            {manifest.busy ? "Updating…" : "Update game database"}
          </Button>
        </div>
        <SwitchField
          label="Keep it up to date automatically"
          hint="Ludusavi checks once a day while backing up. Turning this off keeps the database you have and leaves updating to the button."
          checked={settings.data?.saveManifestAutoUpdate ?? true}
          onChange={(next) => save({ saveManifestAutoUpdate: next })}
        />
        <SaveError error={manifest.error ?? error} />
        <p className={HINT}>
          This is the list of where games keep their saves, and it is updated far more
          often than Ludusavi itself. Update it when a game of yours is not recognised. A
          game that is in no version of the list needs its save folder set by hand, on the
          game's row in Saves.
        </p>
      </div>
    </VersionSection>
  );
}

/** What this machine is called beside its saves. */
export function DeviceNameField() {
  const settings = useAppSettings();
  const { save, error } = useSettingSaver();
  const detected = useQuery({
    queryKey: ["detected-device-name"],
    queryFn: () => backend.detectedDeviceName(),
    staleTime: Infinity,
  });

  return (
    <>
      <TextField
        label="This device's name"
        value={settings.data?.deviceName ?? ""}
        placeholder={detected.data ?? "This PC"}
        onCommit={(value) => void save({ deviceName: value })}
      />
      <SaveError error={error} />
      <p className={`pt-1 ${HINT}`}>
        Shown beside every save this machine uploads, so you can tell which one a save came
        from. Leave it empty to use the name the system reports.
      </p>
    </>
  );
}
