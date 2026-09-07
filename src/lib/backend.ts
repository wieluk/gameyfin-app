/**
 * The single seam between the UI and the Rust core.
 *
 * In a packaged app every call is a Tauri command. Running `vite dev` in a plain browser
 * (no Tauri) falls back to fixtures, so the interface can be developed and reviewed
 * without a server or a GUI toolchain present.
 */

import type { LibraryEntry, Library } from "@/types";
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
  serverUrl: string | null;
  username: string | null;
  libraryRoot: string | null;
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

export interface WineProgress {
  receivedBytes: number;
  totalBytes: number;
  bytesPerSecond: number;
}

export interface Backend {
  listLibraries(): Promise<Library[]>;
  listEntries(): Promise<LibraryEntry[]>;
  startDownload(gameId: number): Promise<void>;
  installOptions(gameId: number): Promise<InstallPlan>;
  install(gameId: number, method?: string, deleteArchive?: boolean): Promise<void>;
  rescanLibrary(): Promise<number>;
  locateInstall(gameId: number, path: string): Promise<void>;
  copyToClipboard(text: string): Promise<void>;
  uninstall(gameId: number, runUninstaller?: boolean): Promise<void>;
  deleteStaging(gameId: number): Promise<void>;
  deleteDownload(gameId: number): Promise<void>;
  openFolder(gameId: number): Promise<void>;
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
  getSettings(): Promise<{
    logLevel: string;
    libraryRoot: string | null;
    installerMemoryLimitMb: number;
    downloadLimitKib: number;
    wineVariant: WineVariant;
  }>;
  wineStatus(): Promise<WineStatus>;
  /** Download and install Wine, replacing any existing build. Also used to update. */
  installWine(): Promise<InstalledWine>;
  removeWine(): Promise<void>;
  setWineVariant(variant: WineVariant): Promise<void>;
  setDownloadLimit(kib: number): Promise<void>;
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
  logDirectory(): Promise<string>;
  imageCacheSize(): Promise<number>;
  clearImageCache(): Promise<void>;
  setLogLevel(level: string): Promise<void>;
  runSetup(gameId: number, relative: string): Promise<void>;
  setLibraryRoot(path: string): Promise<void>;
}

const tauriBackend: Backend = {
  listLibraries: () => invoke<Library[]>("list_libraries"),
  listEntries: () => invoke<LibraryEntry[]>("list_entries"),
  startDownload: (gameId) => invoke("start_download", { gameId }),
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
  uninstall: (gameId, runUninstaller) =>
    invoke("uninstall_game", { gameId, runUninstaller: runUninstaller ?? true }),
  deleteStaging: (gameId) => invoke("delete_staging", { gameId }),
  deleteDownload: (gameId) => invoke("delete_download", { gameId }),
  openFolder: (gameId) => invoke("open_game_folder", { gameId }),
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
  logDirectory: () => invoke<string>("log_directory"),
  imageCacheSize: () => invoke<number>("image_cache_size"),
  clearImageCache: () => invoke("clear_image_cache"),
  setLogLevel: (level) => invoke("set_log_level", { level }),
  runSetup: (gameId, relative) => invoke("run_setup", { gameId, relative }),
  setLibraryRoot: (path) => invoke("set_library_root", { path }),
};

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
  deleteStaging: async () => {},
  deleteDownload: async (gameId) => console.info(`[mock] delete ${gameId}`),
  openFolder: async (gameId) => console.info(`[mock] open folder ${gameId}`),
  openPath: async (path) => console.info(`[mock] open ${path}`),
  launch: async (gameId) => console.info(`[mock] launch ${gameId}`),
  listExecutables: async () => [],
  setExecutable: async () => {},

  // Outside Tauri the wizard is skipped: there is no window to sign in with.
  connectionStatus: async () => ({
    configured: true,
    authenticated: true,
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
  setDownloadLimit: async () => {},
  cancelDownload: async (gameId) => console.info(`[mock] cancel download ${gameId}`),
  configDirectory: async () => "/tmp/gameyfin",
  prefixInfo: async () => null,
  clearPrefixes: async () => {},
  setInstallerMemoryLimit: async () => {},
  suggestLibraryRoot: async () => "/games",
  pickFolder: async () => null,
  pickFile: async () => null,
  runSetupPath: async () => {},
  logDirectory: async () => "/tmp/gameyfin/logs",
  imageCacheSize: async () => 0,
  clearImageCache: async () => {},
  setLogLevel: async (level) => console.info(`[mock] log level ${level}`),
  runSetup: async () => {},
  setLibraryRoot: async (path) => console.info(`[mock] library root ${path}`),
};

export const backend: Backend = inTauri() ? tauriBackend : mockBackend;
export const isMockBackend = !inTauri();
