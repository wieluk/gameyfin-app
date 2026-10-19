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
        let width = thumbnail_width(request.uri().query());
        responder.respond(fetch(&app, &path, width).await);
    });
}

/// Widths a cover is scaled to for the library grid. The page asks for the smallest one at least
/// as wide as the card on screen, so a thumbnail is never stretched into blur.
const THUMBNAIL_WIDTHS: [u32; 6] = [320, 480, 640, 960, 1280, 1920];

/// The `w` a request asks for, if it is one of [`THUMBNAIL_WIDTHS`].
fn thumbnail_width(query: Option<&str>) -> Option<u32> {
    query?
        .split('&')
        .find_map(|pair| pair.strip_prefix("w="))?
        .parse()
        .ok()
        .filter(|width| THUMBNAIL_WIDTHS.contains(width))
}

async fn fetch(app: &AppHandle, path: &str, width: Option<u32>) -> Response<Vec<u8>> {
    let state = app.state::<AppState>();
    let found = match width {
        Some(width) => thumbnail(&state, path, width).await,
        None => artwork(&state, path).await,
    };
    match found {
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

    let request = state
        .settings()
        .credentials()
        .apply(state.http().get(url.as_str()));
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

/// Artwork scaled down to `width`, cached beside the original. A decoded cover costs its full
/// size in memory however small it is drawn, and a library holds hundreds.
pub async fn thumbnail(
    state: &AppState,
    path: &str,
    width: u32,
) -> Result<(Vec<u8>, String), (StatusCode, &'static str)> {
    let cache = state
        .image_cache()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "still starting"))?;
    let key = format!("{}@{width}w", path.trim_start_matches('/'));
    if let Some(cached) = cache.get(&key).await {
        return Ok(cached);
    }
    let (bytes, content_type) = artwork(state, path).await?;
    let original = bytes.clone();
    // A few at a time: a first scroll asks for hundreds at once, and decoding them all side by
    // side leaves the allocator holding the peak long after.
    static SCALING: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);
    let _turn = SCALING.acquire().await;
    if let Some(cached) = cache.get(&key).await {
        return Ok(cached);
    }
    match tokio::task::spawn_blocking(move || scale(&bytes, width)).await {
        Ok(Some((scaled, scaled_type))) => {
            cache.put(&key, &scaled, scaled_type).await;
            Ok((scaled, scaled_type.to_string()))
        }
        // Already small enough, or not an image this can read: the original serves.
        _ => Ok((original, content_type)),
    }
}

/// `bytes` scaled to `width`, or `None` when it is no wider than that already.
fn scale(bytes: &[u8], width: u32) -> Option<(Vec<u8>, &'static str)> {
    use image::ImageDecoder;

    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let (original_width, original_height) = reader.into_decoder().ok()?.dimensions();
    if original_width <= width {
        return None;
    }
    let height = (u64::from(original_height) * u64::from(width) / u64::from(original_width)).max(1);
    let scaled = image::load_from_memory(bytes).ok()?.resize_exact(
        width,
        height as u32,
        image::imageops::FilterType::Lanczos3,
    );
    let mut out = Vec::new();
    if scaled.color().has_alpha() {
        scaled
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .ok()?;
        Some((out, "image/png"))
    } else {
        // This encoder needs 98 to stay clear of visible loss; the file is still smaller.
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 98)
            .encode_image(&scaled.to_rgb8())
            .ok()?;
        Some((out, "image/jpeg"))
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
    fn only_known_thumbnail_widths_are_honoured() {
        assert_eq!(thumbnail_width(Some("w=480")), Some(480));
        assert_eq!(thumbnail_width(Some("x=1&w=1280")), Some(1280));
        assert_eq!(thumbnail_width(Some("w=481")), None);
        assert_eq!(thumbnail_width(Some("w=big")), None);
        assert_eq!(thumbnail_width(None), None);
    }

    fn jpeg(width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut out)
            .encode_image(&image::RgbImage::new(width, height))
            .unwrap();
        out
    }

    #[test]
    fn a_cover_is_scaled_down_keeping_its_shape() {
        let (scaled, kind) = scale(&jpeg(600, 900), 320).unwrap();
        assert_eq!(kind, "image/jpeg");
        let decoded = image::load_from_memory(&scaled).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (320, 480));
    }

    #[test]
    fn a_cover_is_never_scaled_up() {
        assert!(scale(&jpeg(300, 450), 320).is_none());
        assert!(scale(b"not an image", 320).is_none());
    }

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
