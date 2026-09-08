//! Downloads, extraction and install management.
//!
//! Deliberately free of GUI dependencies so it builds and tests anywhere.

pub mod arguments;
pub mod checkpoint;
pub mod download;
pub mod error;
pub mod executable;
pub mod extract;
pub mod install;
pub mod installer;
pub mod launch;
pub mod payload;
pub mod prefix;
pub mod process;
pub mod runtime;
pub mod save_migration;
pub mod save_store;
pub mod save_sync;
pub mod save_tool;
pub mod shortcuts;
pub mod steam;
pub mod umu;
pub mod wine;

pub use checkpoint::Checkpoint;
pub use download::{DownloadOutcome, Downloader, Progress, StartMode};
pub use error::{CoreError, CoreResult};
pub use executable::{
    detect, find_uninstaller, looks_like_uninstaller, looks_like_windows_program, Detection,
};
pub use extract::{extract, extract_with, ArchiveKind, ExtractProgress, TarCompression};
pub use install::InstallLayout;
pub use installer::{identify, InstallerKind};
pub use launch::{
    needs_proton, resolve_command, AddressSpaceCap, LaunchConfig, ResolvedCommand, Runtime,
};
pub use payload::{classify, methods_for, InstallMethod, Payload};
pub use prefix::{
    dpi_for_scale, games_drive_path, map_drive, map_drive_letter, windows_safe_name, wine_root,
};
pub use process::{
    run_capturing, run_capturing_limited, run_elevated, CapturedRun, Session, SessionEnd,
    Supervisor,
};
pub use runtime::{
    detect_windows_runtime, detect_windows_runtime_in, windows_runtime_hint, WindowsRuntime,
};
pub use save_store::{FolderStore, SaveStore, ServerStore, StoreResult, WebDavStore};
pub use save_sync::{decide, ConflictChoice, LocalSaveState, SaveSync, SaveSyncState};
