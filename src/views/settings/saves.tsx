import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { backend } from "@/lib/backend";
import { formatBytes, formatRelative } from "@/lib/format";
import { keys, useAppSettings, useInvalidate, useSaveToolStatus } from "@/lib/queries";
import { useAction } from "@/lib/useAction";
import { useTauriEvent } from "@/lib/useTauriEvent";
import { HINT, INPUT } from "@/lib/ui";
import type { MigrationSummary } from "@/bindings/MigrationSummary";
import type { SaveBackend } from "@/types";
import { VersionSection, type VersionTool } from "./VersionSection";
import { Check, Field, Row, SaveError, Section, useSettingSaver } from "./controls";

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

  async function browse() {
    const chosen = await backend.pickFolder(data?.saveFolder ?? undefined);
    if (chosen) await update({ saveFolder: chosen });
  }

  return (
    <>
      <Check
        label="Sync my saves"
        hint="Backs up your saves after you play so another PC can pick them up."
        checked={enabled}
        onChange={(next) => update({ saveSyncEnabled: next })}
      />
      {/* Indented and greyed out together, because neither does anything on its own. */}
      <div className="ml-[1.375rem] flex flex-col gap-2 border-l border-default-200/60 pl-3">
        <Check
          label="Restore before a game starts"
          hint="Fetches a newer save from another PC before launching, so you carry on where you left off."
          checked={data?.syncSavesOnLaunch ?? true}
          disabled={!enabled}
          onChange={(next) => update({ syncSavesOnLaunch: next })}
        />
        <Check
          label="Back up after a game closes"
          hint="Uploads your save when you finish playing. Nothing is uploaded if it has not changed."
          checked={data?.syncSavesOnExit ?? true}
          disabled={!enabled}
          onChange={(next) => update({ syncSavesOnExit: next })}
        />
      </div>

      <div className="mt-4 flex flex-col gap-2">
        <p className="text-xs font-medium text-foreground/70">Where saves are kept</p>
        {(
          [
            ["server", "Needs a server with save sync turned on."],
            ["folder", "Any folder something else syncs: Syncthing, rclone, a NextCloud or Dropbox folder."],
            ["webdav", "A NextCloud, ownCloud or other WebDAV share, without mounting it first."],
          ] as Array<[SaveBackend, string]>
        ).map(([id, hint]) => (
          <label key={id} className="flex cursor-pointer items-start gap-2">
            <input
              type="radio"
              name="save-backend"
              className="mt-1"
              checked={active === id}
              onChange={() => update({ saveBackend: id })}
            />
            <span>
              <span className="text-sm">{BACKEND_LABELS[id]}</span>
              <span className="block text-[11px] text-foreground/50">{hint}</span>
            </span>
          </label>
        ))}
      </div>

      {active === "folder" && (
        <div className="mt-3 flex flex-col gap-2">
          <div className="flex items-center gap-2">
            <input
              className="min-w-0 flex-1 rounded-lg border border-default-200 bg-content1 px-3 py-1.5 text-xs"
              placeholder="No folder chosen"
              readOnly
              value={data?.saveFolder ?? ""}
            />
            <button
              type="button"
              onClick={browse}
              className="rounded-lg bg-default-100 px-3 py-1.5 text-xs font-medium hover:bg-default-200"
            >
              Browse
            </button>
          </div>
          {/* The sandbox only reaches the home directory and removable media, the same
              limit the games folder already has. */}
          <p className="text-[11px] text-foreground/50">
            In the Flatpak build the folder has to be inside your home directory.
          </p>
        </div>
      )}

      {active === "webdav" && (
        <div className="mt-3 flex flex-col gap-2">
          <Field
            label="Address"
            placeholder="https://cloud.example.com/remote.php/dav/files/me/saves"
            value={data?.webdavUrl ?? ""}
            onCommit={(value) => update({ webdavUrl: value })}
          />
          <Field
            label="Username"
            value={data?.webdavUsername ?? ""}
            onCommit={(value) => update({ webdavUsername: value })}
          />
          <Field
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
        </div>
      )}

      {/* Directly under the location, since that is what it checks. */}
      <div className="mt-3 flex items-center gap-3">
        <button
          type="button"
          onClick={() => void test.run(async () => setResult(await backend.testSaveStore()))}
          disabled={test.busy}
          className="rounded-lg bg-default-100 px-3 py-1.5 text-xs font-medium hover:bg-default-200 disabled:opacity-50"
        >
          {test.busy ? "Checking…" : "Test connection"}
        </button>
        {result && <span className="text-xs text-success-600">{result}</span>}
        {test.error && <span className="text-xs text-danger">{test.error}</span>}
      </div>
      <SaveError error={error} />

      {active !== "server" && (
        <div className="mt-3">
          <Field
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

/** Copying saves across after changing where they are kept. */
export function MigrationSection() {
  const settings = useAppSettings();
  const active: SaveBackend = settings.data?.saveBackend ?? "server";

  // Defaults to copying into the place saves are kept now, the usual reason to be here.
  // Both ends are still selectable, so a folder can be the target while the server is in
  // use, which is the case for anyone setting a folder up before switching to it.
  const [to, setTo] = useState<SaveBackend>(active);
  const [from, setFrom] = useState<SaveBackend>(active === "server" ? "folder" : "server");
  const [allVersions, setAllVersions] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [summary, setSummary] = useState<MigrationSummary | null>(null);
  const migration = useAction();
  const busy = migration.busy;

  // Switching backend while this is on screen should move the target with it.
  useEffect(() => {
    setTo(active);
    setFrom((current) => (current === active ? BACKENDS.find((b) => b !== active)! : current));
  }, [active]);

  // Subscribed only while a migration is running.
  useTauriEvent<{ done: number; total: number }>("save-migration-progress", setProgress, busy);

  async function run() {
    setSummary(null);
    setProgress(null);
    const result = await migration.run(() => backend.migrateSaves(from, to, allVersions));
    setProgress(null);
    if (result) setSummary(result);
  }

  return (
    <Section title="Move saves">
      <p className={HINT}>
        Nothing is removed from the place you copy from, so you can run this again, and a
        second run only copies what is missing.
      </p>

      <div className="grid grid-cols-2 gap-2 pt-2">
        <label className="flex flex-col gap-1">
          <span className="text-xs text-foreground/55">Copy from</span>
          <select
            value={from}
            onChange={(e) => setFrom(e.target.value as SaveBackend)}
            className={INPUT}
          >
            {BACKENDS.map((backendId) => (
              <option key={backendId} value={backendId}>
                {BACKEND_LABELS[backendId]}
              </option>
            ))}
          </select>
        </label>
        <label className="flex flex-col gap-1">
          <span className="text-xs text-foreground/55">Copy to</span>
          <select
            value={to}
            onChange={(e) => setTo(e.target.value as SaveBackend)}
            className={INPUT}
          >
            {BACKENDS.map((backendId) => (
              <option key={backendId} value={backendId}>
                {BACKEND_LABELS[backendId]}
              </option>
            ))}
          </select>
        </label>
      </div>
      <p className={HINT}>
        Each location keeps its own settings, so both ends work whichever one is in use now.
      </p>

      <label className="flex cursor-pointer items-start gap-2 pt-2">
        <input
          type="checkbox"
          className="mt-1"
          checked={allVersions}
          onChange={(e) => setAllVersions(e.target.checked)}
        />
        <span>
          <span className="text-sm">Copy every version</span>
          <span className="block text-[11px] text-foreground/50">
            Off by default, which copies only each game's newest save.
          </span>
        </span>
      </label>

      <div className="flex flex-wrap items-center gap-3 pt-1">
        <button
          type="button"
          disabled={busy || from === to}
          onClick={() => void run()}
          className="rounded-lg bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground disabled:opacity-50"
        >
          {busy ? "Copying…" : "Copy saves"}
        </button>
        {busy && progress && (
          <span className="text-xs text-foreground/55">
            {progress.done} of {progress.total} games
          </span>
        )}
      </div>

      {from === to && (
        <p className={HINT}>
          Pick two different places to copy between.
        </p>
      )}

      <SaveError error={migration.error} />

      {summary && (
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

      <div className="mt-2 border-t border-default-200 pt-3">
        <Row
          label="Game database"
          value={
            helper?.manifest
              ? `${formatRelative(helper.manifest.updatedAt)}, ${formatBytes(helper.manifest.bytes)}`
              : "Not downloaded yet"
          }
        />
        <div className="flex flex-wrap items-center gap-3 pt-2">
          <button
            type="button"
            disabled={manifest.busy}
            onClick={() => void updateManifest()}
            className="rounded-lg border border-default-200 px-3 py-1.5 text-xs font-medium disabled:opacity-50"
          >
            {manifest.busy ? "Updating…" : "Update game database"}
          </button>
        </div>
        <label className="flex cursor-pointer items-start gap-2 pt-2">
          <input
            type="checkbox"
            className="mt-1"
            checked={settings.data?.saveManifestAutoUpdate ?? true}
            onChange={(e) => void save({ saveManifestAutoUpdate: e.target.checked })}
          />
          <span>
            <span className="text-xs">Keep it up to date automatically</span>
            <span className="block text-[11px] text-foreground/50">
              Ludusavi checks once a day while backing up. Turning this off keeps the
              database you have and leaves updating to the button.
            </span>
          </span>
        </label>
        <SaveError error={manifest.error ?? error} />
        <p className="pt-2 text-[11px] leading-relaxed text-foreground/45">
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
      <Field
        label="This device's name"
        value={settings.data?.deviceName ?? ""}
        placeholder={detected.data ?? "This PC"}
        onCommit={(value) => void save({ deviceName: value })}
      />
      <SaveError error={error} />
      <p className="pt-1 text-[11px] leading-relaxed text-foreground/45">
        Shown beside every save this machine uploads, so you can tell which one a save came
        from. Leave it empty to use the name the system reports.
      </p>
    </>
  );
}
