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
  | { kind: "failed"; message: string; stage: Stage };

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
}
