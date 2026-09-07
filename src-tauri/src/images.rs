//! Serving game artwork to the webview.
//!
//! Artwork cannot simply be linked to directly. Gameyfin's `/images/**` controller carries
//! `@DynamicPublicAccess`, so on an instance that does not allow anonymous browsing it
//! answers 401, and an `<img>` tag cannot authenticate, because the session cookies live
//! in this process rather than in the webview's own jar.
//!
//! So the app registers its own URI scheme. The webview asks for `gfimg://…`, this fetches
//! the bytes with the authenticated client, and hands them back. The webview never needs
//! credentials, and the content security policy stays narrow.

use tauri::http::{Request, Response, StatusCode};
use tauri::{AppHandle, Manager, UriSchemeContext, UriSchemeResponder, Wry};

use crate::state::AppState;

/// The scheme the webview requests artwork over.
pub const SCHEME: &str = "gfimg";

/// Build a URL the webview can load for a server-side image path.
///
/// Windows serves custom schemes over `http://<scheme>.localhost`, while Linux and macOS
/// use the scheme directly; the difference is baked in here so callers need not care.
pub fn url_for(path: &str) -> String {
    let path = path.trim_start_matches('/');
    if cfg!(windows) {
        format!("http://{SCHEME}.localhost/{path}")
    } else {
        format!("{SCHEME}://localhost/{path}")
    }
}

/// Handle one artwork request.
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
    let state = app.state::<AppState>();

    let Some(client) = state.client().await else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "not connected");
    };

    // Only artwork may be requested through this scheme. Without this check the webview
    // could reach any authenticated endpoint on the server by guessing a path.
    if !path.starts_with("images/") {
        return error(StatusCode::FORBIDDEN, "only image paths are served here");
    }

    // Serve from disk when we already have it: artwork never changes for a given id, and
    // refetching the whole grid on every launch is slow and pointless.
    let cache = state.image_cache().await;
    if let Some((bytes, content_type)) = cache.get(path).await {
        return Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", content_type)
            .header("Cache-Control", "public, max-age=86400")
            .body(bytes)
            .unwrap_or_else(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "bad cache entry"));
    }

    let url = client.url_for(&format!("/{path}"));
    let cookies = state.settings().await.cookies;
    let cookie_header = cookies
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ");

    let mut request = state.http().await.get(&url);
    if !cookie_header.is_empty() {
        request = request.header(reqwest::header::COOKIE, cookie_header);
    }

    match request.send().await {
        Ok(response) if response.status().is_success() => {
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("image/jpeg")
                .to_string();

            match response.bytes().await {
                Ok(bytes) => {
                    cache.put(path, &bytes, &content_type).await;
                    Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", content_type)
                        // Artwork for a given id never changes, so let the webview keep it.
                        .header("Cache-Control", "public, max-age=86400")
                        .body(bytes.to_vec())
                        .unwrap_or_else(|_| {
                            error(StatusCode::INTERNAL_SERVER_ERROR, "bad response")
                        })
                }
                Err(e) => {
                    tracing::debug!("could not read image body for {path}: {e}");
                    error(StatusCode::BAD_GATEWAY, "could not read image")
                }
            }
        }
        Ok(response) => {
            // Logged because a wall of missing artwork is otherwise hard to explain.
            tracing::warn!("image {path} returned HTTP {}", response.status());
            error(StatusCode::NOT_FOUND, "image unavailable")
        }
        Err(e) => {
            tracing::warn!("could not fetch image {path}: {e}");
            error(StatusCode::BAD_GATEWAY, "could not reach the server")
        }
    }
}

fn error(status: StatusCode, message: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "text/plain")
        .body(message.as_bytes().to_vec())
        .expect("static error response is valid")
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
    }

    #[test]
    fn tolerates_a_missing_leading_slash() {
        assert!(url_for("images/cover/7").ends_with("/images/cover/7"));
    }
}
