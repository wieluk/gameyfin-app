//! Downloads, extraction and install management.
//!
//! Deliberately free of GUI dependencies so it builds and tests anywhere.

pub mod arguments;
pub mod checkpoint;
pub mod diagnose;
pub mod download;
pub mod environment;
pub mod error;
pub mod executable;
pub mod extract;
pub mod graphics;
pub mod icon;
pub mod install;
pub mod installer;
pub mod launch;
pub mod payload;
pub mod prefix;
pub mod process;
pub mod proton;
pub mod runtime;
pub mod save_migration;
pub mod save_store;
pub mod save_sync;
pub mod save_tool;
pub mod setups;
pub mod shortcuts;
pub mod steam;
pub mod umu;
pub mod unarc;
pub mod vulkan;
pub mod wine;
pub mod zip_stream;

pub use checkpoint::Checkpoint;
pub use download::{DownloadOutcome, Downloader, Progress, StartMode};
pub use error::{CoreError, CoreResult};
pub use executable::{
    detect, find_uninstaller, looks_like_uninstaller, looks_like_windows_program, Detection,
};
pub use extract::{extract, extract_with, ArchiveKind, ExtractProgress, TarCompression};
pub use graphics::{Component, InstalledGraphics};
pub use install::InstallLayout;
pub use installer::{identify, InstallerKind};
pub use launch::{needs_proton, resolve_command, LaunchConfig, ResolvedCommand, Runtime};
pub use payload::{classify, methods_for, InstallMethod, Payload};
pub use prefix::{
    dpi_for_screen, games_drive_path, map_drive, map_drive_letter, windows_safe_name, wine_root,
    DllOverrides, PrefixState,
};
pub use process::{
    run_capturing, run_capturing_limited, run_capturing_stoppable, run_elevated, CapturedRun,
    Session, SessionEnd, Stopper, Supervisor,
};
pub use runtime::{detect_windows_runtime, windows_runtime_hint, RuntimeContext, WindowsRuntime};
pub use save_store::{FolderStore, SaveStore, ServerStore, StoreResult, WebDavStore};
pub use save_sync::{decide, ConflictChoice, LocalSaveState, SaveSync, SaveSyncState};
pub use vulkan::{GraphicsTier, VulkanSupport};
