//! Save synchronisation against the server's `/saves` REST routes.
//!
//! Archives are binary, so this is plain REST rather than Hilla RPC, matching the server.

use std::path::Path;

use reqwest::multipart::{Form, Part};
use reqwest::{Body, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;

use crate::client::GameyfinClient;
use crate::error::{ApiError, ApiResult};

/// One version of a game's saves as the server describes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveVersion {
    /// Opaque to everything but the store that issued it. The Gameyfin server uses a
    /// database id, a folder or WebDAV store uses a filename, so this is a string and
    /// nothing may assume it is a number.
    #[serde(deserialize_with = "lenient_id")]
    pub id: String,
    pub game_id: i64,
    #[serde(default)]
    pub game_title: Option<String>,
    pub size_bytes: u64,
    pub content_hash: String,
    #[serde(default)]
    pub platform: String,
    #[serde(default)]
    pub installation_id: Option<String>,
    #[serde(default)]
    pub device_name: Option<String>,
    #[serde(default)]
    pub ludusavi_title: Option<String>,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub created_at: Option<String>,
}

/// What the client claims about an upload. Mirrors the `X-` headers the server reads.
#[derive(Debug, Clone, Default)]
pub struct UploadMetadata {
    pub content_hash: String,
    pub platform: String,
    pub installation_id: Option<String>,
    pub device_name: Option<String>,
    pub ludusavi_title: Option<String>,
    /// The version this upload was based on. A mismatch is what the server calls a conflict.
    pub base_save_id: Option<String>,
    /// Upload anyway, keeping the version that would otherwise have won.
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum UploadOutcome {
    Stored(Box<SaveVersion>),
    /// The server already holds these exact bytes.
    Unchanged,
    Conflict {
        remote: Box<SaveVersion>,
        base_save_id: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConflictBody {
    remote: SaveVersion,
    #[serde(default, deserialize_with = "lenient_optional_id")]
    base_save_id: Option<String>,
}

/// Accepts an id written as either a JSON number or a string.
///
/// The Gameyfin server sends a number, other stores send a filename, and records written
/// before this became opaque still hold a number.
fn lenient_id<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::String(text) => text,
        serde_json::Value::Number(number) => number.to_string(),
        other => other.to_string(),
    })
}

pub fn lenient_optional_id<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Ok(
        match Option::<serde_json::Value>::deserialize(deserializer)? {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(text)) => Some(text),
            Some(serde_json::Value::Number(number)) => Some(number.to_string()),
            Some(other) => Some(other.to_string()),
        },
    )
}

/// SHA-256 of a file, hex encoded, read in chunks so a large archive stays off the heap.
pub async fn hash_file(path: &Path) -> std::io::Result<String> {
    use tokio::io::AsyncReadExt;

    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];

    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

impl GameyfinClient {
    /// Versions for a game, newest first.
    ///
    /// This doubles as the capability probe. Listing cannot legitimately 404 on a server
    /// that has the feature, since an unknown game simply has no versions, so a 404 here
    /// means the route does not exist and the server predates save sync.
    pub async fn list_saves(&self, game_id: i64) -> ApiResult<Vec<SaveVersion>> {
        let url = self.url_for(&format!("/saves/game/{game_id}"));
        let req = self.auth().apply(self.http().get(&url)).await?;
        let resp = req.send().await?;
        let status = resp.status();
        let body = resp.text().await?;

        if status == StatusCode::NOT_FOUND {
            return Err(ApiError::SaveSyncUnsupported);
        }
        check_status("list_saves", status, &body)?;
        serde_json::from_str(&body).map_err(|source| ApiError::Decode {
            endpoint: "list_saves".into(),
            source,
        })
    }

    /// Streams a version to `destination`. Returns the number of bytes written.
    pub async fn download_save(
        &self,
        game_id: i64,
        save_id: &str,
        destination: &Path,
    ) -> ApiResult<u64> {
        self.download_from(&format!("/saves/game/{game_id}/{save_id}"), destination)
            .await
    }

    pub async fn download_latest_save(&self, game_id: i64, destination: &Path) -> ApiResult<u64> {
        self.download_from(&format!("/saves/game/{game_id}/latest"), destination)
            .await
    }

    async fn download_from(&self, path: &str, destination: &Path) -> ApiResult<u64> {
        use futures_util::StreamExt;

        let url = self.url_for(path);
        let req = self.auth().apply(self.http().get(&url)).await?;
        let resp = req.send().await?;
        let status = resp.status();

        if !status.is_success() {
            let body = resp.text().await?;
            return Err(status_error("download_save", status, &body));
        }

        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io_error)?;
        }

        let mut file = tokio::fs::File::create(destination)
            .await
            .map_err(io_error)?;
        let mut written = 0u64;
        let mut stream = resp.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await.map_err(io_error)?;
            written += chunk.len() as u64;
        }
        file.flush().await.map_err(io_error)?;

        Ok(written)
    }

    pub async fn upload_save(
        &self,
        game_id: i64,
        archive: &Path,
        metadata: &UploadMetadata,
    ) -> ApiResult<UploadOutcome> {
        let url = self.url_for(&format!("/saves/game/{game_id}"));

        let file = tokio::fs::File::open(archive).await.map_err(io_error)?;
        let length = file.metadata().await.map_err(io_error)?.len();
        let part = Part::stream_with_length(Body::wrap_stream(ReaderStream::new(file)), length)
            .file_name("save.zip")
            .mime_str("application/zip")
            .map_err(ApiError::Transport)?;

        let mut req = self
            .http()
            .post(&url)
            .header("X-Content-Hash", &metadata.content_hash)
            .multipart(Form::new().part("file", part));

        if !metadata.platform.is_empty() {
            req = req.header("X-Save-Platform", &metadata.platform);
        }
        if let Some(id) = &metadata.installation_id {
            req = req.header("X-Installation-Id", id);
        }
        if let Some(name) = &metadata.device_name {
            req = req.header("X-Device-Name", name);
        }
        if let Some(title) = &metadata.ludusavi_title {
            req = req.header("X-Ludusavi-Title", title);
        }
        if let Some(base) = &metadata.base_save_id {
            req = req.header("X-Base-Save-Id", base);
        }
        if metadata.force {
            req = req.header("X-Force", "true");
        }

        let resp = self.auth().apply(req).await?.send().await?;
        let status = resp.status();
        let body = resp.text().await?;

        match status {
            StatusCode::NO_CONTENT => Ok(UploadOutcome::Unchanged),
            StatusCode::CONFLICT => {
                let conflict: ConflictBody =
                    serde_json::from_str(&body).map_err(|source| ApiError::Decode {
                        endpoint: "upload_save".into(),
                        source,
                    })?;
                Ok(UploadOutcome::Conflict {
                    remote: Box::new(conflict.remote),
                    base_save_id: conflict.base_save_id,
                })
            }
            _ => {
                check_status("upload_save", status, &body)?;
                let stored: SaveVersion =
                    serde_json::from_str(&body).map_err(|source| ApiError::Decode {
                        endpoint: "upload_save".into(),
                        source,
                    })?;
                Ok(UploadOutcome::Stored(Box::new(stored)))
            }
        }
    }

    /// Marks a version exempt from retention pruning.
    ///
    /// Metadata rather than bytes, so this is the one save operation that goes over Hilla.
    pub async fn set_save_locked(&self, save_id: &str, locked: bool) -> ApiResult<()> {
        let id: i64 = save_id
            .parse()
            .map_err(|_| ApiError::Other(format!("not a server save id: {save_id}")))?;
        self.call::<Option<serde_json::Value>>(
            "SaveSyncEndpoint",
            "setLocked",
            serde_json::json!({ "saveId": id, "locked": locked }),
        )
        .await?;
        Ok(())
    }

    pub async fn delete_save(&self, game_id: i64, save_id: &str) -> ApiResult<()> {
        self.delete_at(&format!("/saves/game/{game_id}/{save_id}"))
            .await
    }

    pub async fn delete_all_saves(&self, game_id: i64) -> ApiResult<()> {
        self.delete_at(&format!("/saves/game/{game_id}")).await
    }

    async fn delete_at(&self, path: &str) -> ApiResult<()> {
        let url = self.url_for(path);
        let req = self.auth().apply(self.http().delete(&url)).await?;
        let resp = req.send().await?;
        let status = resp.status();
        let body = resp.text().await?;

        check_status("delete_save", status, &body)
    }
}

fn io_error(source: std::io::Error) -> ApiError {
    ApiError::Other(source.to_string())
}

fn check_status(endpoint: &str, status: StatusCode, body: &str) -> ApiResult<()> {
    if status.is_success() {
        Ok(())
    } else {
        Err(status_error(endpoint, status, body))
    }
}

/// Maps the statuses the save routes add on top of the usual ones.
fn status_error(endpoint: &str, status: StatusCode, body: &str) -> ApiError {
    match status {
        StatusCode::METHOD_NOT_ALLOWED => ApiError::SaveSyncDisabled,
        StatusCode::PAYLOAD_TOO_LARGE => ApiError::QuotaExceeded,
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
            ApiError::Unauthenticated(endpoint.to_string())
        }
        s => ApiError::Status {
            endpoint: endpoint.to_string(),
            status: s.as_u16(),
            body: body.chars().take(200).collect(),
        },
    }
}
