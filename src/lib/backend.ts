/**
 * The single seam between the UI and the Rust core.
 *
 * In a packaged app every call is a Tauri command. Running `vite dev` in a plain browser
 * (no Tauri) falls back to fixtures, so the interface can be developed and reviewed
 * without a server or a GUI toolchain present.
 */

import type {
  ConflictChoice,
  LibraryEntry,
  Library,
  SaveSyncState,
  SaveVersion,
} from "@/types";
import { mockEntries, mockLibraries } from "./fixtures";

/** Tauri injects this before any app code runs. */
function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke: tauriInvoke } = await import("@tauri-apps/api/core");
  return tauriInvoke<T>(cmd, args);
}

export interface ConnectionStatus {
  configured: boolean;
  authenticated: boolean;
  /** The server did not answer. The session is kept and the cached library is shown. */
  offline: boolean;
  serverUrl: string | null;
  username: string | null;
  libraryRoot: string | null;
}

/** One of the two folders the app owns inside the games folder. */
export type LibraryFolder = "downloads" | "installations";

/** Where a game's shortcut can be put. */
export type ShortcutLocation = "desktop" | "menu";

export interface ShortcutStatus {
  desktop: boolean;
  menu: boolean;
  /** False when Steam is not installed, in which case the option is not offered. */
  steamAvailable: boolean;
  steam: boolean;
}

/** How this copy of the app can be updated. See `updater.rs`. */
export type UpdateChannel = "self-install" | "flatpak" | "system-package" | "development";

export interface UpdateStatus {
  currentVersion: string;
  latestVersion: string | null;
  available: boolean;
  channel: UpdateChannel;
  /** Whether the app can install the update itself, or can only point at it. */
  canInstall: boolean;
  releaseUrl: string;
  notes: string | null;
  /** Why the check failed, when it did. */
  error: string | null;
}

export interface PrefixEntry {
  gameId: number;
  /** Null for a prefix whose game is no longer in the library. */
  title: string | null;
  path: string;
  bytes: number;
}

/** A Wine tool that can be pointed at one prefix. */
export type PrefixTool = "winecfg" | "explorer" | "regedit";

/** One configured games folder. */
export interface LibraryRoot {
  path: string;
  /** The folder new downloads go to unless told otherwise. */
  isDefault: boolean;
  /** Free space on its drive, or null when that cannot be read. */
  freeBytes: number | null;
  /** False when the folder is gone, so the UI can say so rather than failing later. */
  exists: boolean;
}

/** Extra options a single game is given. */
export interface GameOptions {
  launchArguments: string;
  installerArguments: string;
}

/** Which palette the interface uses. */
export type Theme = "system" | "light" | "dark";

export interface UmuStatus {
  entries: number;
  enabled: boolean;
  /** The id a launch would use for the game that was asked about. */
  resolved: string | null;
}

export interface ServerProbe {
  url: string;
  reachable: boolean;
  authenticated: boolean;
  message: string | null;
}

export interface LoginPoll {
  signedIn: boolean;
  windowOpen: boolean;
  detail: string | null;
}

/** Emitted by the sign-in window as it moves between origins. */
export interface LoginProgress {
  host: string;
  onServer: boolean;
}

export interface InstallOption {
  key: string;
  label: string;
  description: string;
  interactive: boolean;
  /** Set when the option cannot be used yet, explaining what is missing. */
  blockedBy: string | null;
}

export interface InstallPlan {
  payload: string;
  options: InstallOption[];
  defaultInstallDir: string;
  /** Path to type into a setup wizard, when one will ask. */
  windowsInstallPath: string | null;
  /** Whether any offered option hands over to a setup wizard. */
  needsInstallPath: boolean;
  /** Where a file picker should open for this game. */
  browseDir: string | null;
  setupCandidates: string[];
}

/** Which Wine build to download. `staging-wow64` needs no 32-bit host libraries. */
export type WineVariant = "staging-wow64" | "staging";

export interface InstalledWine {
  version: string;
  variant: WineVariant;
  binary: string;
}

export interface WineRelease {
  version: string;
  variant: WineVariant;
  asset: string;
  url: string;
  sizeBytes: number;
  sha256: string | null;
}

export interface WineStatus {
  installed: InstalledWine | null;
  /** Null when the release feed could not be reached, which is not the same as up to date. */
  latest: WineRelease | null;
}

/** Everything stored in `settings.json` that the interface can change. */
export interface AppSettings {
  logLevel: string;
  libraryRoot: string | null;
  installerMemoryLimitMb: number;
  downloadLimitKib: number;
  wineVariant: WineVariant;
  /** The user turned down the startup offer to download Wine. Linux only. */
  winePromptDismissed: boolean;
  notifyTransfers: boolean;
  notifyFailures: boolean;
  notifyUpdates: boolean;
  closeToTray: boolean;
  startMinimized: boolean;
  autoInstall: boolean;
  umuFixes: boolean;
  gamepadEnabled: boolean;
  gamepadDeadzone: number;
  couchModeAuto: boolean;
  checkForUpdates: boolean;
  extractionPassword: string | null;
  ignoredExecutables: string[];
  theme: Theme;
  autostart: boolean;
  saveSyncEnabled: boolean;
  syncSavesOnLaunch: boolean;
  syncSavesOnExit: boolean;
}

/** A download provider the server offers, with the one this client uses marked. */
export interface ProviderChoice {
  key: string;
  name: string;
  description: string;
  selected: boolean;
  /** True when this provider serves a `.torrent` rather than the game itself. */
  needsTorrentClient: boolean;
}

export interface WineProgress {
  receivedBytes: number;
  totalBytes: number;
  bytesPerSecond: number;
}

export interface Backend {
  listLibraries(): Promise<Library[]>;
  listEntries(): Promise<LibraryEntry[]>;
  /** `root` must be one of the configured games folders; omitted means the default. */
  startDownload(gameId: number, root?: string): Promise<void>;
  installOptions(gameId: number): Promise<InstallPlan>;
  install(gameId: number, method?: string, deleteArchive?: boolean): Promise<void>;
  rescanLibrary(): Promise<number>;
  locateInstall(gameId: number, path: string): Promise<void>;
  copyToClipboard(text: string): Promise<void>;
  /** `uninstaller` overrides detection, for a game whose uninstaller is oddly named. */
  uninstall(gameId: number, runUninstaller?: boolean, uninstaller?: string | null): Promise<void>;
  /** The uninstaller in the game's folder, or null when nothing looks like one. */
  findUninstaller(gameId: number): Promise<string | null>;
  deleteStaging(gameId: number): Promise<void>;
  deleteDownload(gameId: number): Promise<void>;
  /** `folder` picks which of the game's folders to reveal; Downloads by default there. */
  openFolder(gameId: number, folder?: LibraryFolder): Promise<void>;
  /** Reveal the Downloads or Installations folder itself, not one game's. */
  openLibraryFolder(folder: LibraryFolder, root?: string): Promise<void>;
  openPath(path: string): Promise<void>;
  launch(gameId: number): Promise<void>;
  listExecutables(gameId: number): Promise<string[]>;
  setExecutable(gameId: number, executable: string): Promise<void>;

  connectionStatus(): Promise<ConnectionStatus>;
  probeServer(url: string): Promise<ServerProbe>;
  setServerUrl(url: string): Promise<string>;
  /** `direct` forces the password form on a server that also has SSO. */
  beginLogin(direct?: boolean): Promise<void>;
  pollLogin(): Promise<LoginPoll>;
  cancelLogin(): Promise<void>;
  resetLogin(): Promise<void>;
  signOut(): Promise<void>;
  getSettings(): Promise<AppSettings>;
  wineStatus(): Promise<WineStatus>;
  /** Download and install Wine, replacing any existing build. Also used to update. */
  installWine(): Promise<InstalledWine>;
  removeWine(): Promise<void>;
  setWineVariant(variant: WineVariant): Promise<void>;
  /** Stop offering Wine at startup. */
  setWinePromptDismissed(dismissed: boolean): Promise<void>;
  setDownloadLimit(kib: number): Promise<void>;
  /** What the server can download from, and which one is in use. */
  downloadProviders(): Promise<ProviderChoice[]>;
  /** Null restores the server’s own preference order. */
  setDownloadProvider(key: string | null): Promise<void>;
  /** Stop a running download. The partial file is kept, but Gameyfin 2.4 cannot resume. */
  cancelDownload(gameId: number): Promise<void>;
  configDirectory(): Promise<string>;
  prefixInfo(): Promise<{ path: string; count: number; bytes: number } | null>;
  clearPrefixes(): Promise<void>;
  setInstallerMemoryLimit(megabytes: number): Promise<void>;
  suggestLibraryRoot(): Promise<string>;
  /** Native folder picker; null when the user cancels. */
  pickFolder(current?: string): Promise<string | null>;
  /** Native file picker; null when the user cancels. */
  pickFile(startIn?: string): Promise<string | null>;
  runSetupPath(gameId: number, path: string): Promise<void>;
  /** Retry the setup program Windows refused to start, as administrator. */
  runSetupElevated(gameId: number): Promise<void>;
  logDirectory(): Promise<string>;
  imageCacheSize(): Promise<number>;
  clearImageCache(): Promise<void>;
  setLogLevel(level: string): Promise<void>;
  runSetup(gameId: number, relative: string): Promise<void>;
  setLibraryRoot(path: string): Promise<void>;

  setNotificationOptions(transfers: boolean, failures: boolean, updates: boolean): Promise<void>;
  setWindowOptions(closeToTray: boolean, startMinimized: boolean): Promise<void>;
  setAutoInstall(enabled: boolean): Promise<void>;
  setGamepadOptions(enabled: boolean, deadzone: number, couchModeAuto: boolean): Promise<void>;
  setUmuFixes(enabled: boolean): Promise<void>;
  setUpdateChecking(enabled: boolean): Promise<void>;
  /** Quit for real. The close button may be set to hide the window instead. */
  quitApp(): Promise<void>;

  shortcutStatus(gameId: number): Promise<ShortcutStatus>;
  setShortcut(gameId: number, location: ShortcutLocation, enabled: boolean): Promise<void>;
  /** Returns a sentence to show, because Steam has to be restarted to see the change. */
  setSteamShortcut(gameId: number, enabled: boolean): Promise<string>;

  listPrefixes(): Promise<PrefixEntry[]>;
  deletePrefix(gameId: number): Promise<void>;
  openPrefixTool(gameId: number, tool: PrefixTool): Promise<void>;

  umuStatus(gameId?: number): Promise<UmuStatus>;
  refreshUmuDatabase(): Promise<number>;

  /** Where one game's saves stand, without changing anything. */
  saveState(gameId: number): Promise<SaveSyncState>;
  listSaveVersions(gameId: number): Promise<SaveVersion[]>;
  /** Back up and upload. `force` accepts a stale base, keeping the losing version. */
  backupSaves(gameId: number, force: boolean): Promise<SaveSyncState>;
  /** Restore a version, newest if none is named. */
  restoreSaves(gameId: number, saveId?: string): Promise<SaveSyncState>;
  resolveSaveConflict(gameId: number, choice: ConflictChoice): Promise<SaveSyncState>;
  /** Name the title Ludusavi should use, for a game it could not identify. */
  setSaveTitle(gameId: number, title: string | null): Promise<SaveSyncState>;
  /** Choose how saves map onto this machine, plus any hand-written path pairs. */
  setSaveMapping(
    gameId: number,
    crossOs: boolean,
    redirects: Array<[string, string]>,
  ): Promise<SaveSyncState>;
  deleteSaveVersion(gameId: number, saveId: string): Promise<void>;
  setSaveSyncSettings(enabled: boolean, onLaunch: boolean, onExit: boolean): Promise<void>;

  updateStatus(): Promise<UpdateStatus>;
  /** Returns what to tell the user; what happens next differs by package format. */
  installUpdate(): Promise<string>;

  listLibraryRoots(): Promise<LibraryRoot[]>;
  addLibraryRoot(path: string): Promise<void>;
  /** Forgets the folder. Nothing on disk is touched. */
  removeLibraryRoot(path: string): Promise<void>;
  setDefaultLibraryRoot(path: string): Promise<void>;

  gameOptions(gameId: number): Promise<GameOptions>;
  setGameOptions(
    gameId: number,
    launchArguments: string,
    installerArguments: string,
  ): Promise<void>;

  setExtractionOptions(password: string | null, ignoredExecutables: string[]): Promise<void>;
  setTheme(theme: Theme): Promise<void>;
  setAutostart(enabled: boolean): Promise<void>;
}

const tauriBackend: Backend = {
  listLibraries: () => invoke<Library[]>("list_libraries"),
  listEntries: () => invoke<LibraryEntry[]>("list_entries"),
  startDownload: (gameId, root) => invoke("start_download", { gameId, root: root ?? null }),
  installOptions: (gameId) => invoke<InstallPlan>("install_options", { gameId }),
  install: (gameId, method, deleteArchive) =>
    invoke("install_game", {
      gameId,
      method: method ?? null,
      deleteArchive: deleteArchive ?? null,
    }),
  rescanLibrary: () => invoke<number>("rescan_library"),
  locateInstall: (gameId, path) => invoke("locate_install", { gameId, path }),
  copyToClipboard: async (text) => {
    const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
    await writeText(text);
  },
  uninstall: (gameId, runUninstaller, uninstaller) =>
    invoke("uninstall_game", {
      gameId,
      runUninstaller: runUninstaller ?? true,
      uninstaller: uninstaller ?? null,
    }),
  findUninstaller: (gameId) => invoke<string | null>("find_game_uninstaller", { gameId }),
  deleteStaging: (gameId) => invoke("delete_staging", { gameId }),
  deleteDownload: (gameId) => invoke("delete_download", { gameId }),
  openFolder: (gameId, folder) => invoke("open_game_folder", { gameId, folder: folder ?? null }),
  openLibraryFolder: (folder, root) =>
    invoke("open_library_folder", { folder, root: root ?? null }),
  openPath: (path) => invoke("open_path", { path }),
  launch: (gameId) => invoke("launch_game", { gameId }),
  listExecutables: (gameId) => invoke<string[]>("list_executables", { gameId }),
  setExecutable: (gameId, executable) => invoke("set_game_executable", { gameId, executable }),

  connectionStatus: () => invoke<ConnectionStatus>("connection_status"),
  probeServer: (url) => invoke<ServerProbe>("probe_server", { url }),
  setServerUrl: (url) => invoke<string>("set_server_url", { url }),
  beginLogin: (direct) => invoke("begin_login", { direct: direct ?? false }),
  pollLogin: () => invoke<LoginPoll>("poll_login"),
  resetLogin: () => invoke("reset_login"),
  cancelLogin: () => invoke("cancel_login"),
  signOut: () => invoke("sign_out"),
  getSettings: () => invoke("get_settings"),
  setDownloadLimit: (kib) => invoke("set_download_limit", { kib }),
  downloadProviders: () => invoke<ProviderChoice[]>("download_providers"),
  setDownloadProvider: (key) => invoke("set_download_provider", { key }),
  cancelDownload: (gameId) => invoke("cancel_download", { gameId }),
  configDirectory: () => invoke<string>("config_directory"),
  prefixInfo: () => invoke("prefix_info"),
  clearPrefixes: () => invoke("clear_prefixes"),
  setInstallerMemoryLimit: (megabytes) =>
    invoke("set_installer_memory_limit", { megabytes }),
  wineStatus: () => invoke("wine_status"),
  installWine: () => invoke("install_wine"),
  removeWine: () => invoke("remove_wine"),
  setWineVariant: (variant) => invoke("set_wine_variant", { variant }),
  setWinePromptDismissed: (dismissed) =>
    invoke("set_wine_prompt_dismissed", { dismissed }),
  suggestLibraryRoot: () => invoke<string>("suggest_library_root"),
  pickFolder: async (current) => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const chosen = await open({ directory: true, multiple: false, defaultPath: current });
    return typeof chosen === "string" ? chosen : null;
  },
  pickFile: async (startIn) => {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const chosen = await open({
      directory: false,
      multiple: false,
      defaultPath: startIn,
      filters: [
        { name: "Programs", extensions: ["exe", "msi", "sh", "AppImage", "x86_64"] },
        { name: "All files", extensions: ["*"] },
      ],
    });
    return typeof chosen === "string" ? chosen : null;
  },
  runSetupPath: (gameId, path) => invoke("run_setup_path", { gameId, path }),
  runSetupElevated: (gameId) => invoke("run_setup_elevated", { gameId }),
  logDirectory: () => invoke<string>("log_directory"),
  imageCacheSize: () => invoke<number>("image_cache_size"),
  clearImageCache: () => invoke("clear_image_cache"),
  setLogLevel: (level) => invoke("set_log_level", { level }),
  runSetup: (gameId, relative) => invoke("run_setup", { gameId, relative }),
  setLibraryRoot: (path) => invoke("set_library_root", { path }),

  setNotificationOptions: (transfers, failures, updates) =>
    invoke("set_notification_options", { transfers, failures, updates }),
  setWindowOptions: (closeToTray, startMinimized) =>
    invoke("set_window_options", { closeToTray, startMinimized }),
  setAutoInstall: (enabled) => invoke("set_auto_install", { enabled }),
  setGamepadOptions: (enabled, deadzone, couchModeAuto) =>
    invoke("set_gamepad_options", { enabled, deadzone, couchModeAuto }),
  setUmuFixes: (enabled) => invoke("set_umu_fixes", { enabled }),
  setUpdateChecking: (enabled) => invoke("set_update_checking", { enabled }),
  quitApp: () => invoke("quit_app"),

  shortcutStatus: (gameId) => invoke<ShortcutStatus>("shortcut_status", { gameId }),
  setShortcut: (gameId, location, enabled) =>
    invoke("set_shortcut", { gameId, location, enabled }),
  setSteamShortcut: (gameId, enabled) =>
    invoke<string>("set_steam_shortcut", { gameId, enabled }),

  listPrefixes: () => invoke<PrefixEntry[]>("list_prefixes"),
  deletePrefix: (gameId) => invoke("delete_prefix", { gameId }),
  openPrefixTool: (gameId, tool) => invoke("open_prefix_tool", { gameId, tool }),

  umuStatus: (gameId) => invoke<UmuStatus>("umu_status", { gameId: gameId ?? null }),
  refreshUmuDatabase: () => invoke<number>("refresh_umu_database"),

  saveState: (gameId) => invoke<SaveSyncState>("save_state", { gameId }),
  listSaveVersions: (gameId) => invoke<SaveVersion[]>("list_save_versions", { gameId }),
  backupSaves: (gameId, force) => invoke<SaveSyncState>("backup_saves", { gameId, force }),
  restoreSaves: (gameId, saveId) => invoke<SaveSyncState>("restore_saves", { gameId, saveId }),
  resolveSaveConflict: (gameId, choice) =>
    invoke<SaveSyncState>("resolve_save_conflict", { gameId, choice }),
  setSaveTitle: (gameId, title) => invoke<SaveSyncState>("set_save_title", { gameId, title }),
  setSaveMapping: (gameId, crossOs, redirects) =>
    invoke<SaveSyncState>("set_save_mapping", { gameId, crossOs, redirects }),
  deleteSaveVersion: (gameId, saveId) =>
    invoke<void>("delete_save_version", { gameId, saveId }),
  setSaveSyncSettings: (enabled, onLaunch, onExit) =>
    invoke<void>("set_save_sync_settings", { enabled, onLaunch, onExit }),

  updateStatus: () => invoke<UpdateStatus>("update_status"),
  installUpdate: () => invoke<string>("install_update"),

  listLibraryRoots: () => invoke<LibraryRoot[]>("list_library_roots"),
  addLibraryRoot: (path) => invoke("add_library_root", { path }),
  removeLibraryRoot: (path) => invoke("remove_library_root", { path }),
  setDefaultLibraryRoot: (path) => invoke("set_default_library_root", { path }),

  gameOptions: (gameId) => invoke<GameOptions>("game_options", { gameId }),
  setGameOptions: (gameId, launchArguments, installerArguments) =>
    invoke("set_game_options", { gameId, launchArguments, installerArguments }),

  setExtractionOptions: (password, ignoredExecutables) =>
    invoke("set_extraction_options", { password, ignoredExecutables }),
  setTheme: (theme) => invoke("set_theme", { theme }),
  setAutostart: (enabled) => invoke("set_autostart", { enabled }),
};

function mockSaveVersion(gameId: number): SaveVersion {
  return {
    id: "1",
    gameId,
    gameTitle: "Fixture Game",
    sizeBytes: 2_400_000,
    contentHash: "a".repeat(64),
    platform: "WINDOWS",
    deviceName: "desktop",
    locked: false,
    createdAt: new Date().toISOString(),
  };
}

const mockBackend: Backend = {
  listLibraries: async () => mockLibraries,
  listEntries: async () => mockEntries,
  startDownload: async (gameId) => console.info(`[mock] download ${gameId}`),
  installOptions: async () => ({
    payload: "zip",
    options: [
      {
        key: "extract",
        label: "Extract",
        description: "Unpack the archive into your games folder.",
        interactive: false,
        blockedBy: null,
      },
    ],
    defaultInstallDir: "/games/Gameyfin/Installations/(1) Demo",
    windowsInstallPath: null,
    needsInstallPath: false,
    browseDir: null,
    setupCandidates: [],
  }),
  install: async (gameId, method) => console.info(`[mock] install ${gameId} (${method})`),
  rescanLibrary: async () => 0,
  locateInstall: async () => {},
  copyToClipboard: async () => {},
  uninstall: async (gameId) => console.info(`[mock] uninstall ${gameId}`),
  findUninstaller: async () => null,
  deleteStaging: async () => {},
  deleteDownload: async (gameId) => console.info(`[mock] delete ${gameId}`),
  openFolder: async (gameId) => console.info(`[mock] open folder ${gameId}`),
  openLibraryFolder: async (folder) => console.info(`[mock] open ${folder} folder`),
  openPath: async (path) => console.info(`[mock] open ${path}`),
  launch: async (gameId) => console.info(`[mock] launch ${gameId}`),
  listExecutables: async () => [],
  setExecutable: async () => {},

  // Outside Tauri the wizard is skipped: there is no window to sign in with.
  connectionStatus: async () => ({
    configured: true,
    authenticated: true,
    offline: false,
    serverUrl: "https://demo.invalid",
    username: "demo",
    libraryRoot: "/games",
  }),
  probeServer: async (url) => ({ url, reachable: true, authenticated: false, message: null }),
  setServerUrl: async (url) => url,
  beginLogin: async (direct) => console.info(`[mock] begin login (direct=${direct})`),
  pollLogin: async () => ({ signedIn: true, windowOpen: false, detail: null }),
  resetLogin: async () => {},
  cancelLogin: async () => {},
  signOut: async () => console.info("[mock] sign out"),
  getSettings: async () => ({
    logLevel: "info",
    libraryRoot: "/games",
    installerMemoryLimitMb: 3072,
    downloadLimitKib: 0,
    wineVariant: "staging-wow64" as WineVariant,
    winePromptDismissed: false,
    notifyTransfers: true,
    notifyFailures: true,
    notifyUpdates: true,
    closeToTray: true,
    startMinimized: false,
    autoInstall: false,
    umuFixes: true,
    gamepadEnabled: true,
    gamepadDeadzone: 0.25,
    couchModeAuto: true,
    checkForUpdates: true,
    extractionPassword: null,
    ignoredExecutables: ["unitycrashhandler", "vcredist"],
    theme: "dark" as Theme,
    autostart: false,
    saveSyncEnabled: true,
    syncSavesOnLaunch: true,
    syncSavesOnExit: true,
  }),
  wineStatus: async () => ({
    installed: { version: "11.17", variant: "staging-wow64" as WineVariant, binary: "/tmp/wine" },
    latest: null,
  }),
  installWine: async () => ({
    version: "11.17",
    variant: "staging-wow64" as WineVariant,
    binary: "/tmp/wine",
  }),
  removeWine: async () => {},
  setWineVariant: async (variant) => console.info(`[mock] wine variant ${variant}`),
  setWinePromptDismissed: async (dismissed) =>
    console.info(`[mock] wine prompt dismissed ${dismissed}`),
  setDownloadLimit: async () => {},
  downloadProviders: async () => [
    {
      key: "org.gameyfin.direct",
      name: "Direct Download",
      description: "Streamed straight from the server.",
      selected: true,
      needsTorrentClient: false,
    },
    {
      key: "org.gameyfin.torrent",
      name: "Torrent",
      description: "Hands back a .torrent file.",
      selected: false,
      needsTorrentClient: true,
    },
  ],
  setDownloadProvider: async (key) => console.info(`[mock] provider ${key}`),
  cancelDownload: async (gameId) => console.info(`[mock] cancel download ${gameId}`),
  configDirectory: async () => "/tmp/gameyfin",
  prefixInfo: async () => null,
  clearPrefixes: async () => {},
  setInstallerMemoryLimit: async () => {},
  suggestLibraryRoot: async () => "/games",
  pickFolder: async () => null,
  pickFile: async () => null,
  runSetupPath: async () => {},
  runSetupElevated: async () => {},
  logDirectory: async () => "/tmp/gameyfin/logs",
  imageCacheSize: async () => 0,
  clearImageCache: async () => {},
  setLogLevel: async (level) => console.info(`[mock] log level ${level}`),
  runSetup: async () => {},
  setLibraryRoot: async (path) => console.info(`[mock] library root ${path}`),

  setNotificationOptions: async () => {},
  setWindowOptions: async () => {},
  setAutoInstall: async (enabled) => console.info(`[mock] auto install ${enabled}`),
  setGamepadOptions: async () => {},
  setUmuFixes: async () => {},
  setUpdateChecking: async () => {},
  quitApp: async () => console.info("[mock] quit"),

  shortcutStatus: async () => ({
    desktop: false,
    menu: true,
    steamAvailable: true,
    steam: false,
  }),
  setShortcut: async (gameId, location, enabled) =>
    console.info(`[mock] shortcut ${location} ${gameId} ${enabled}`),
  setSteamShortcut: async () => "Added to Steam. Restart Steam to see it in your library.",

  listPrefixes: async () => [
    { gameId: 1, title: "Celeste", path: "/games/Gameyfin/Prefixes/1", bytes: 620_000_000 },
    { gameId: 7, title: null, path: "/games/Gameyfin/Prefixes/7", bytes: 410_000_000 },
  ],
  deletePrefix: async (gameId) => console.info(`[mock] delete prefix ${gameId}`),
  openPrefixTool: async (gameId, tool) => console.info(`[mock] ${tool} for ${gameId}`),

  umuStatus: async () => ({ entries: 3120, enabled: true, resolved: "umu-504230" }),
  refreshUmuDatabase: async () => 3120,

  listLibraryRoots: async () => [
    { path: "/games", isDefault: true, freeBytes: 512_000_000_000, exists: true },
    { path: "/mnt/big", isDefault: false, freeBytes: 2_400_000_000_000, exists: true },
  ],
  addLibraryRoot: async (path) => console.info(`[mock] add root ${path}`),
  removeLibraryRoot: async (path) => console.info(`[mock] remove root ${path}`),
  setDefaultLibraryRoot: async (path) => console.info(`[mock] default root ${path}`),

  gameOptions: async () => ({ launchArguments: "", installerArguments: "" }),
  setGameOptions: async () => {},

  setExtractionOptions: async () => {},
  setTheme: async (theme) => console.info(`[mock] theme ${theme}`),
  setAutostart: async (enabled) => console.info(`[mock] autostart ${enabled}`),

  saveState: async (gameId) =>
    gameId % 3 === 0
      ? { kind: "conflict", localAt: new Date().toISOString(), remote: mockSaveVersion(gameId) }
      : { kind: "in-sync", lastSyncedAt: new Date().toISOString() },
  listSaveVersions: async (gameId) => [mockSaveVersion(gameId)],
  backupSaves: async (gameId) => {
    console.info(`[mock] back up saves for ${gameId}`);
    return { kind: "in-sync", lastSyncedAt: new Date().toISOString() };
  },
  restoreSaves: async (gameId) => {
    console.info(`[mock] restore saves for ${gameId}`);
    return { kind: "in-sync", lastSyncedAt: new Date().toISOString() };
  },
  resolveSaveConflict: async (gameId, choice) => {
    console.info(`[mock] resolve ${gameId} as ${choice}`);
    return { kind: "in-sync", lastSyncedAt: new Date().toISOString() };
  },
  setSaveTitle: async () => ({ kind: "never-synced" }),
  setSaveMapping: async () => ({ kind: "never-synced" }),
  deleteSaveVersion: async (gameId, saveId) =>
    console.info(`[mock] delete save ${saveId} of ${gameId}`),
  setSaveSyncSettings: async (enabled) => console.info(`[mock] save sync ${enabled}`),

  updateStatus: async () => ({
    currentVersion: "0.1.0",
    latestVersion: "0.1.0",
    available: false,
    channel: "development" as UpdateChannel,
    canInstall: false,
    releaseUrl: "https://github.com/gameyfin/gameyfin-app/releases/latest",
    notes: null,
    error: null,
  }),
  installUpdate: async () => "Nothing to update outside Tauri.",
};

export const backend: Backend = inTauri() ? tauriBackend : mockBackend;
export const isMockBackend = !inTauri();
