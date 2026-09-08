//! Client for the Gameyfin server API.
//!
//! Targets Gameyfin 2.4.0+. See `docs/01-desktop-app-plan.md` for how this fits the app,
//! and `docs/02-save-sync-plan.md` for the server changes the client is designed to adopt.

pub mod auth;
pub mod client;
pub mod error;
pub mod models;
pub mod saves;

pub use auth::{AuthStrategy, CookieSessionAuth, DeviceTokenAuth};
pub use client::GameyfinClient;
pub use error::{ApiError, ApiResult};
pub use models::{DownloadProvider, Game, GameMetadata, Image, Library, UserInfo};
pub use saves::{hash_file, SaveVersion, UploadMetadata, UploadOutcome};
