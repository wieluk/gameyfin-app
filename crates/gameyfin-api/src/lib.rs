//! Client for the Gameyfin server API, targeting Gameyfin 2.4.0+ (and its `/v1` API from 2.6).

pub mod auth;
pub mod catalog;
pub mod client;
pub mod error;
pub mod models;
pub mod saves;
pub mod v1;

pub use auth::{
    cookie_header, ApiKeyAuth, AuthStrategy, CookieSessionAuth, DeviceTokenAuth, API_KEY_HEADER,
};
pub use catalog::GameCatalog;
pub use client::{http_client, GameyfinClient};
pub use error::{ApiError, ApiResult};
pub use models::{DownloadProvider, Game, GameMetadata, Image, Library, UserInfo};
pub use saves::{hash_file, SaveVersion, UploadMetadata, UploadOutcome};
pub use v1::{api_key_id, is_api_key};
