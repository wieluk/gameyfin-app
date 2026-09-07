//! Save game backup and restore for the Gameyfin desktop client.
//!
//! Wraps Ludusavi (MIT, bundled as a Tauri sidecar) and adds the parts a syncing client
//! needs on top: deterministic game identification, and a config that makes backups
//! portable between machines.

pub mod api;
pub mod config;
pub mod error;
pub mod ludusavi;
pub mod resolve;
pub mod runner;

pub use error::{SaveError, SaveResult};
pub use ludusavi::{BackupFormat, GameQuery, Ludusavi};
pub use resolve::{resolve, Candidate, GameIdentity, TitleMatch};
