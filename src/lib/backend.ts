/**
 * The single seam between the UI and the Rust core. In a plain browser (`vite dev`) calls
 * answer from fixtures, so the UI works without a server or GUI toolchain.
 */

import type { ConflictChoice } from "@/bindings/ConflictChoice";
import type { ConnectionStatus } from "@/bindings/ConnectionStatus";
import type { GameOptions } from "@/bindings/GameOptions";
import type { GameOptionsPatch } from "@/bindings/GameOptionsPatch";
import type { InstalledProton } from "@/bindings/InstalledProton";
import type { InstalledSaveTool } from "@/bindings/InstalledSaveTool";
import type { InstallPlan } from "@/bindings/InstallPlan";
import type { Library } from "@/bindings/Library";
import type { LibraryEntry } from "@/bindings/LibraryEntry";
import type { LibraryFolder } from "@/bindings/LibraryFolder";
import type { LibraryRoot } from "@/bindings/LibraryRoot";
import type { Location as ShortcutLocation } from "@/bindings/Location";
import type { LoginPoll } from "@/bindings/LoginPoll";
import type { ManifestInfo } from "@/bindings/ManifestInfo";
import type { MemoryInfo } from "@/bindings/MemoryInfo";
import type { MigrationSummary } from "@/bindings/MigrationSummary";
import type { PrefixEntry } from "@/bindings/PrefixEntry";
import type { PrefixTool } from "@/bindings/PrefixTool";
import type { ProtonFamily } from "@/bindings/ProtonFamily";
import type { ProtonStatus } from "@/bindings/ProtonStatus";
import type { ProviderChoice } from "@/bindings/ProviderChoice";
import type { PublicSettings } from "@/bindings/PublicSettings";
import type { RestoreReport } from "@/bindings/RestoreReport";
import type { SaveBackend } from "@/bindings/SaveBackend";
import type { SaveFind } from "@/bindings/SaveFind";
import type { SaveLocations } from "@/bindings/SaveLocations";
import type { SaveOverviewRow } from "@/bindings/SaveOverviewRow";
import type { SavePathSettings } from "@/bindings/SavePathSettings";
import type { SaveScope } from "@/bindings/SaveScope";
import type { SaveSyncState } from "@/bindings/SaveSyncState";
import type { SaveToolStatus } from "@/bindings/SaveToolStatus";
import type { SaveVersion } from "@/bindings/SaveVersion";
import type { ServerProbe } from "@/bindings/ServerProbe";
import type { SettingsPatch } from "@/bindings/SettingsPatch";
import type { ShortcutStatus } from "@/bindings/ShortcutStatus";
import type { UmuStatus } from "@/bindings/UmuStatus";
import type { UpdateStatus } from "@/bindings/UpdateStatus";

export type {
  LibraryFolder,
  LibraryRoot,
  PrefixEntry,
  ProviderChoice,
  ShortcutLocation,
  UpdateStatus,
};

/** Tauri injects this before any app code runs. */
export const isMockBackend = typeof window === "undefined" || !("__TAURI_INTERNALS__" in window);

async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (isMockBackend) {
    const { mockInvoke } = await import("./mock");
    return mockInvoke<T>(command, args);
  }
  const { invoke: tauriInvoke } = await import("@tauri-apps/api/core");
  return tauriInvoke<T>(command, args);
}

export const backend = {
  listLibraries: () => invoke<Library[]>("list_libraries"),
  listEntries: () => invoke<LibraryEntry[]>("list_entries"),
  /** `root` must be one of the configured games folders; omitted means the default. */
  startDownload: (gameId: number, root?: string) =>
    invoke<void>("start_download", { gameId, root: root ?? null }),
  cancelDownload: (gameId: number) => invoke<void>("cancel_download", { gameId }),
  installOptions: (gameId: number) => invoke<InstallPlan>("install_options", { gameId }),
  install: (gameId: number, method?: string, deleteArchive?: boolean) =>
    invoke<void>("install_game", {
      gameId,
      method: method ?? null,
      deleteArchive: deleteArchive ?? null,
    }),
  rescanLibrary: () => invoke<number>("rescan_library"),
  locateInstall: (gameId: number, path: string) => invoke<void>("locate_install", { gameId, path }),
  /** `uninstaller` overrides detection, for a game whose uninstaller is oddly named. */
  uninstall: (gameId: number, runUninstaller?: boolean, uninstaller?: string | null) =>
    invoke<void>("uninstall_game", {
      gameId,
      runUninstaller: runUninstaller ?? true,
      uninstaller: uninstaller ?? null,
    }),
  findUninstaller: (gameId: number) => invoke<string | null>("find_game_uninstaller", { gameId }),
  deleteStaging: (gameId: number) => invoke<void>("delete_staging", { gameId }),
  deleteDownload: (gameId: number) => invoke<void>("delete_download", { gameId }),
  runSetup: (gameId: number, relative: string) => invoke<void>("run_setup", { gameId, relative }),
  runSetupPath: (gameId: number, path: string) => invoke<void>("run_setup_path", { gameId, path }),
  /** Retry the setup program Windows refused to start, as administrator. */
  runSetupElevated: (gameId: number) => invoke<void>("run_setup_elevated", { gameId }),
  launch: (gameId: number) => invoke<void>("launch_game", { gameId }),
  /** Stop whatever the game is running, an installer or the game itself. */
  stopGame: (gameId: number) => invoke<void>("stop_game", { gameId }),
  listExecutables: (gameId: number) => invoke<string[]>("list_executables", { gameId }),
  setExecutable: (gameId: number, executable: string) =>
    invoke<void>("set_game_executable", { gameId, executable }),
  gameOptions: (gameId: number) => invoke<GameOptions>("game_options", { gameId }),
  setGameOptions: (gameId: number, options: GameOptionsPatch) =>
    invoke<void>("set_game_options", { gameId, options }),

  /** Reveal one of a game's folders; Downloads unless told otherwise. */
  openGameFolder: (gameId: number, folder?: LibraryFolder) =>
    invoke<void>("open_game_folder", { gameId, folder: folder ?? null }),
  /** Reveal the Downloads or Installations folder itself. */
  openLibraryFolder: (folder: LibraryFolder, root?: string) =>
    invoke<void>("open_library_folder", { folder, root: root ?? null }),
  openFolder: (path: string) => invoke<void>("open_folder", { path }),
  openUrl: (url: string) => invoke<void>("open_url", { url }),

  connectionStatus: () => invoke<ConnectionStatus>("connection_status"),
  probeServer: (url: string) => invoke<ServerProbe>("probe_server", { url }),
  setServerUrl: (url: string) => invoke<string>("set_server_url", { url }),
  /** `direct` forces the password form on a server that also has SSO. */
  beginLogin: (direct?: boolean) => invoke<void>("begin_login", { direct: direct ?? false }),
  pollLogin: () => invoke<LoginPoll>("poll_login"),
  cancelLogin: () => invoke<void>("cancel_login"),
  resetLogin: () => invoke<void>("reset_login"),
  signOut: () => invoke<void>("sign_out"),
  /** Quit for real. The close button may be set to hide the window instead. */
  quitApp: () => invoke<void>("quit_app"),

  getSettings: () => invoke<PublicSettings>("get_settings"),
  /** Change some settings. Rejects a value it does not know. */
  updateSettings: (patch: SettingsPatch) => invoke<void>("update_settings", { patch }),
  suggestLibraryRoot: () => invoke<string>("suggest_library_root"),
  listLibraryRoots: () => invoke<LibraryRoot[]>("list_library_roots"),
  addLibraryRoot: (path: string) => invoke<void>("add_library_root", { path }),
  /** Forgets the folder. Nothing on disk is touched. */
  removeLibraryRoot: (path: string) => invoke<void>("remove_library_root", { path }),
  setDefaultLibraryRoot: (path: string) => invoke<void>("set_default_library_root", { path }),
  /** What the server can download from, and which one is in use. */
  downloadProviders: () => invoke<ProviderChoice[]>("download_providers"),
  /** Null restores the server's own preference order. */
  setDownloadProvider: (key: string | null) => invoke<void>("set_download_provider", { key }),
  configDirectory: () => invoke<string>("config_directory"),
  logDirectory: () => invoke<string>("log_directory"),
  imageCacheSize: () => invoke<number>("image_cache_size"),
  clearImageCache: () => invoke<void>("clear_image_cache"),
  memoryInfo: () => invoke<MemoryInfo>("memory_info"),
  /** Put an interface crash in the log file, which is all a packaged build leaves. */
  reportCrash: (details: string) => invoke<void>("report_crash", { details }),

  protonStatus: () => invoke<ProtonStatus>("proton_status"),
  /** Download the newest build of a family, replacing the older one. */
  installProton: (family: ProtonFamily) => invoke<InstalledProton>("install_proton", { family }),
  removeProton: (name: string) => invoke<void>("remove_proton", { name }),
  /** Flatpak only: installs the runtime's 32-bit libraries on the host. */
  install32bitSupport: () => invoke<string>("install_32bit_support"),

  shortcutStatus: (gameId: number) => invoke<ShortcutStatus>("shortcut_status", { gameId }),
  setShortcut: (gameId: number, location: ShortcutLocation, enabled: boolean) =>
    invoke<void>("set_shortcut", { gameId, location, enabled }),
  /** Returns a sentence to show, because Steam has to be restarted to see the change. */
  setSteamShortcut: (gameId: number, enabled: boolean) =>
    invoke<string>("set_steam_shortcut", { gameId, enabled }),
  listPrefixes: () => invoke<PrefixEntry[]>("list_prefixes"),
  deletePrefix: (gameId: number) => invoke<void>("delete_prefix", { gameId }),
  openPrefixTool: (gameId: number, tool: PrefixTool) =>
    invoke<void>("open_prefix_tool", { gameId, tool }),
  /** Returns a sentence to show. Can take minutes while winetricks downloads. */
  runWinetricks: (gameId: number, verbs: string) =>
    invoke<string>("run_winetricks", { gameId, verbs }),
  umuStatus: (gameId?: number) => invoke<UmuStatus>("umu_status", { gameId: gameId ?? null }),
  refreshUmuDatabase: () => invoke<number>("refresh_umu_database"),

  /** Where one game's saves stand, without changing anything. */
  saveState: (gameId: number) => invoke<SaveSyncState>("save_state", { gameId }),
  /** Every game's save status in one call. Never runs the backup helper. */
  saveOverview: (scope: SaveScope) => invoke<SaveOverviewRow[]>("save_overview", { scope }),
  listSaveVersions: (gameId: number) => invoke<SaveVersion[]>("list_save_versions", { gameId }),
  /** Back up and upload. `force` accepts a stale base, keeping the losing version. */
  backupSaves: (gameId: number, force: boolean) =>
    invoke<SaveSyncState>("backup_saves", { gameId, force }),
  /** Restore a version, newest if none is named. Says where the files went. */
  restoreSaves: (gameId: number, saveId?: string) =>
    invoke<RestoreReport>("restore_saves", { gameId, saveId }),
  /** Delete stored versions for good. */
  deleteSaveVersions: (gameId: number, saveIds: string[]) =>
    invoke<void>("delete_save_versions", { gameId, saveIds }),
  /** Keep a version safe from pruning, or let it be pruned again. */
  setSaveLocked: (gameId: number, saveId: string, locked: boolean) =>
    invoke<void>("set_save_locked", { gameId, saveId, locked }),
  /** Every stored save, for every game. Answers how many were deleted. */
  deleteAllSaves: () => invoke<number>("delete_all_saves"),
  /** Answers the first-start offer: the version to restore, or null to keep this PC's save. */
  answerSavePullOffer: (gameId: number, saveId: string | null) =>
    invoke<void>("answer_save_pull_offer", { gameId, saveId }),
  resolveSaveConflict: (gameId: number, choice: ConflictChoice) =>
    invoke<SaveSyncState>("resolve_save_conflict", { gameId, choice }),
  /** Search the save manifest for what the user typed. Best match first. */
  searchSaveTitles: (gameId: number, query: string) =>
    invoke<string[]>("search_save_titles", { gameId, query }),
  /** Name the title Ludusavi should use, for a game it could not identify. */
  setSaveTitle: (gameId: number, title: string | null) =>
    invoke<SaveSyncState>("set_save_title", { gameId, title }),
  savePaths: (gameId: number) => invoke<SavePathSettings>("save_paths", { gameId }),
  /** Folders worth opening or browsing for a game. `probe` scans for where saves really are. */
  saveLocations: (gameId: number, probe = false) =>
    invoke<SaveLocations>("save_locations", { gameId, probe }),
  /** Every save the helper can find on this PC, whether or not Gameyfin installed the game. */
  scanThisPc: () => invoke<SaveFind[]>("scan_this_pc"),
  /** Stops the automatic sync a game is waiting on, if it has not passed the point of no return. */
  skipSaveSync: (gameId: number) => invoke<void>("skip_save_sync", { gameId }),
  /** Turn Windows/Linux path translation on for one game, leaving its paths alone. */
  setSaveCrossOs: (gameId: number, crossOs: boolean) =>
    invoke<SaveSyncState>("set_save_cross_os", { gameId, crossOs }),
  setSaveMapping: (
    gameId: number,
    crossOs: boolean,
    redirects: Array<[string, string]>,
    customPaths: string[],
  ) => invoke<SaveSyncState>("set_save_mapping", { gameId, crossOs, redirects, customPaths }),
  /** Copy saves from another location into the one in use. The source is left alone. */
  migrateSaves: (from: SaveBackend, allVersions: boolean) =>
    invoke<MigrationSummary>("migrate_saves", { from, allVersions }),
  saveToolStatus: () => invoke<SaveToolStatus>("save_tool_status"),
  /** Refresh the game database. It changes far more often than the helper itself. */
  updateSaveManifest: () => invoke<ManifestInfo>("update_save_manifest"),
  /** The host name, for the placeholder when no name has been chosen. */
  detectedDeviceName: () => invoke<string | null>("detected_device_name"),
  /** Install a version of the backup helper, or the newest when none is named. */
  installSaveTool: (version?: string) =>
    invoke<InstalledSaveTool>("install_save_tool", { version }),
  removeSaveTool: () => invoke<void>("remove_save_tool"),
  /** Checks the configured location answers. Returns a sentence to show the user. */
  testSaveStore: () => invoke<string>("test_save_store"),

  updateStatus: () => invoke<UpdateStatus>("update_status"),
  /** Returns what to tell the user; what happens next differs by package format. */
  installUpdate: () => invoke<string>("install_update"),

  async copyToClipboard(text: string): Promise<void> {
    if (isMockBackend) return;
    const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
    await writeText(text);
  },
  /** Native folder picker; null when the user cancels. */
  async pickFolder(current?: string): Promise<string | null> {
    return pick({ directory: true, defaultPath: current });
  },
  /** Native file picker; null when the user cancels. */
  async pickFile(startIn?: string): Promise<string | null> {
    return pick({
      directory: false,
      defaultPath: startIn,
      filters: [
        { name: "Programs", extensions: ["exe", "msi", "sh", "AppImage", "x86_64"] },
        { name: "All files", extensions: ["*"] },
      ],
    });
  },
};

async function pick(options: Record<string, unknown>): Promise<string | null> {
  if (isMockBackend) return null;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const chosen = await open({ multiple: false, ...options });
  return typeof chosen === "string" ? chosen : null;
}
