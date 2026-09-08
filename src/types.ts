/** Mirrors `gameyfin_api::models` on the Rust side. Keep the two in step. */

export interface Image {
  id: number;
  type: string;
  blurhash?: string | null;
}

export interface GameMetadata {
  fileSize: number;
  /** Plugin id -> that provider's own id. Absent on a stock 2.4.0 server. */
  originalIds?: Record<string, string> | null;
}

export interface Game {
  id: number;
  title: string;
  libraryId: number;
  summary?: string | null;
  release?: string | null;
  userRating?: number | null;
  criticRating?: number | null;
  platforms: string[];
  genres: string[];
  developers: string[];
  publishers: string[];
  cover?: Image | null;
  header?: Image | null;
  metadata: GameMetadata;
}

export interface Library {
  id: number;
  name: string;
  gameIds: number[];
}

export type Stage = "download" | "extract" | "install" | "launch";

/** Mirrors `library_state::GameState` on the Rust side. */
export type GameState =
  | { kind: "not-installed" }
  | { kind: "downloading"; receivedBytes: number; totalBytes: number; bytesPerSecond: number }
  /** Archive on disk, not yet unpacked. */
  | { kind: "downloaded"; archivePath: string; bytes: number }
  | { kind: "extracting"; percent: number }
  /** Unpacked in the Downloads folder, awaiting a decision about installing. */
  | { kind: "extracted"; path: string; setupCandidates: string[]; archivePresent: boolean }
  | { kind: "installing"; percent: number }
  /** Setting up the compatibility layer; the first run downloads Proton. */
  | { kind: "preparing"; message: string }
  | {
      kind: "installed";
      path: string;
      executable?: string | null;
      setupCandidates?: string[];
      /** Setup programs still in the unpacked download, such as a DLC installer. */
      stagingSetups?: string[];
      /** Whether unpacked files are still taking up space in Downloads. */
      stagingPresent?: boolean;
    }
  | { kind: "running"; since: string }
  | {
      kind: "failed";
      message: string;
      stage: Stage;
      /** Windows refused to start the installer without administrator rights. */
      elevationRequired?: boolean;
    };

export interface LibraryEntry {
  game: Game;
  state: GameState;
  minutesPlayed: number;
  lastPlayedAt?: string | null;
  /** True when a downloaded archive is still on disk for this game. */
  archivePresent?: boolean;
  /** Absolute artwork URLs resolved by the backend, which knows the server address. */
  coverUrl?: string | null;
  headerUrl?: string | null;
  screenshotUrls?: string[];
  /** Gameplay videos, as the server recorded them. Usually YouTube links. */
  videoUrls?: string[];
}

/** Mirrors `gameyfin_saves::SavePlatform` on the Rust side. */
export type SavePlatform = "WINDOWS" | "LINUX" | "PROTON" | "MACOS" | "UNKNOWN";

/** One version of a game's saves as the server holds it. */
export interface SaveVersion {
  /** Opaque: a database id on a Gameyfin server, a filename in a folder store. */
  id: string;
  gameId: number;
  gameTitle?: string | null;
  sizeBytes: number;
  contentHash: string;
  platform: SavePlatform;
  installationId?: string | null;
  deviceName?: string | null;
  ludusaviTitle?: string | null;
  locked: boolean;
  createdAt?: string | null;
}

/** Mirrors `save_sync::SaveSyncState` on the Rust side. Keep the two in step. */
export type SaveSyncState =
  /** The server has no save sync at all, so it predates the feature. */
  | { kind: "unsupported" }
  /** The server could sync saves but an administrator has switched it off. */
  | { kind: "disabled" }
  /** Ludusavi does not recognise this game, so there is nothing to back up yet. */
  | { kind: "unmatched"; candidates: string[] }
  | { kind: "never-synced" }
  /** The helper ran and found no save files, as opposed to never having been tried. */
  | { kind: "nothing-to-back-up" }
  | { kind: "in-sync"; lastSyncedAt?: string | null }
  | { kind: "local-newer"; localAt?: string | null }
  | { kind: "remote-newer"; remoteAt?: string | null; device?: string | null }
  /** Both sides moved since the last sync; only the user can choose. */
  | { kind: "conflict"; localAt?: string | null; remote: SaveVersion }
  /** The newest save was taken somewhere it cannot be restored from directly. */
  | {
      kind: "platform-mismatch";
      local: SavePlatform;
      remote: SavePlatform;
      /** Whether Ludusavi's Wine translation could bridge it, rather than a hand mapping. */
      crossOsAvailable: boolean;
    }
  | { kind: "failed"; message: string };

/** How the user answered a conflict. */
export type ConflictChoice = "keep-local" | "keep-remote" | "keep-both";
