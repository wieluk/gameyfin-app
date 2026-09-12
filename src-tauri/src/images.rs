//! Serving game artwork to the webview over `gfimg://`. An `<img>` tag cannot authenticate
//! to Gameyfin's `/images/**`, since the session cookies live in this process.

use tauri::http::{Request, Response, StatusCode};
use tauri::{AppHandle, Manager, UriSchemeContext, UriSchemeResponder, Wry};

use crate::state::AppState;

pub const SCHEME: &str = "gfimg";

/// Windows serves custom schemes over `http://<scheme>.localhost`; elsewhere the scheme is used.
pub fn url_for(path: &str) -> String {
    let path = path.trim_start_matches('/');
    if cfg!(windows) {
        format!("http://{SCHEME}.localhost/{path}")
    } else {
        format!("{SCHEME}://localhost/{path}")
    }
}

pub fn handle(
    context: UriSchemeContext<'_, Wry>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let app = context.app_handle().clone();
    tauri::async_runtime::spawn(async move {
        let path = request.uri().path().trim_start_matches('/').to_string();
        responder.respond(fetch(&app, &path).await);
    });
}

async fn fetch(app: &AppHandle, path: &str) -> Response<Vec<u8>> {
    match artwork(&app.state::<AppState>(), path).await {
        Ok((bytes, content_type)) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", content_type)
            // Artwork for a given id never changes, so let the webview keep it.
            .header("Cache-Control", "public, max-age=86400")
            .body(bytes)
            .unwrap_or_else(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "bad response")),
        Err((status, message)) => error(status, message),
    }
}

/// One piece of artwork and its content type, from the cache or else the server.
pub async fn artwork(
    state: &AppState,
    path: &str,
) -> Result<(Vec<u8>, String), (StatusCode, &'static str)> {
    let path = path.trim_start_matches('/');
    // Cached artwork is on this disk whether or not the server answers, and a library that
    // renders offline should render with its covers.
    let cache = state
        .image_cache()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "still starting"))?;
    if let Some(cached) = cache.get(path).await {
        return Ok(cached);
    }

    let client = state
        .client()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "not connected"))?;
    // Checked after normalisation, so no escape through `..` or an encoded separator can
    // turn this scheme into a way to reach any authenticated endpoint on the server.
    let url = url::Url::parse(&client.url_for(&format!("/{path}")))
        .map_err(|_| (StatusCode::FORBIDDEN, "not an image path"))?;
    let base =
        url::Url::parse(client.base_url()).map_err(|_| (StatusCode::FORBIDDEN, "no server"))?;
    if url.origin() != base.origin() || !url.path().starts_with("/images/") {
        return Err((StatusCode::FORBIDDEN, "only image paths are served here"));
    }

    let cookies = gameyfin_api::cookie_header(&state.settings().cookies);
    let mut request = state.http().get(url.as_str());
    if !cookies.is_empty() {
        request = request.header(reqwest::header::COOKIE, cookies);
    }
    match request.send().await {
        Ok(response) if response.status().is_success() => {
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("image/jpeg")
                .to_string();
            let bytes = response.bytes().await.map_err(|e| {
                tracing::debug!("could not read image body for {path}: {e}");
                (StatusCode::BAD_GATEWAY, "could not read image")
            })?;
            cache.put(path, &bytes, &content_type).await;
            Ok((bytes.to_vec(), content_type))
        }
        Ok(response) => {
            // Logged because a wall of missing artwork is otherwise hard to explain.
            tracing::warn!("image {path} returned HTTP {}", response.status());
            Err((StatusCode::NOT_FOUND, "image unavailable"))
        }
        Err(e) => {
            tracing::warn!("could not fetch image {path}: {e}");
            Err((StatusCode::BAD_GATEWAY, "could not reach the server"))
        }
    }
}

fn error(status: StatusCode, message: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain")
        .body(message.as_bytes().to_vec())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_platform_appropriate_url() {
        let url = url_for("/images/cover/7");
        if cfg!(windows) {
            assert_eq!(url, "http://gfimg.localhost/images/cover/7");
        } else {
            assert_eq!(url, "gfimg://localhost/images/cover/7");
        }
        assert!(url_for("images/cover/7").ends_with("/images/cover/7"));
    }
}
