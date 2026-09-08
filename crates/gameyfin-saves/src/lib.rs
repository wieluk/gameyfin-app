//! Save backup/restore: wraps Ludusavi and adds deterministic game identification plus
//! machine-portable backup config.

pub mod api;
pub mod config;
pub mod error;
pub mod ludusavi;
pub mod platform;
pub mod resolve;
pub mod runner;

pub use error::{SaveError, SaveResult};
pub use ludusavi::{BackupFormat, GameQuery, Ludusavi};
pub use platform::SavePlatform;
pub use resolve::{resolve, Candidate, GameIdentity, TitleMatch};
