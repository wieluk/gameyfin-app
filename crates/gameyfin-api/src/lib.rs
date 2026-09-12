//! Client for the Gameyfin server API, targeting Gameyfin 2.4.0+.

pub mod auth;
pub mod client;
pub mod error;
pub mod models;
pub mod saves;

pub use auth::{cookie_header, AuthStrategy, CookieSessionAuth, DeviceTokenAuth};
pub use client::GameyfinClient;
pub use error::{ApiError, ApiResult};
pub use models::{DownloadProvider, Game, GameMetadata, Image, Library, UserInfo};
pub use saves::{hash_file, SaveVersion, UploadMetadata, UploadOutcome};
