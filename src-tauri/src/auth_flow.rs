//! First-run connection and sign-in. Gameyfin offers no token endpoint, so we open a real
//! webview on its login page and read the session cookies back out; SSO then needs no
//! special handling since any provider chain happens inside a real browser engine.

use std::collections::HashMap;

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

/// Label of the sign-in window, so it can be found and closed again.
pub const LOGIN_WINDOW: &str = "gameyfin-login";

/// What a probe found at a URL.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerProbe {
    /// The normalised URL the client will use.
    pub url: String,
    /// Whether the server answered as a Gameyfin instance.
    pub reachable: bool,
    /// True when the server already considers this client signed in.
    pub authenticated: bool,
    /// Present when the server is reachable but not usable.
    pub message: Option<String>,
}

/// Normalise user input (bare host, full URL, or a pasted deep link) into a base origin.
pub fn normalize_url(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Detect the scheme on the raw input: stripping a trailing slash first would turn
    // "https://" into "https:" and then be read as a hostname.
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        // Default to HTTPS; http:// still works if typed explicitly.
        format!("https://{trimmed}")
    };

    let parsed = url::Url::parse(&with_scheme).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }

    // Rebuild from parsed parts to drop any path, query or trailing slash.
    let host = parsed.host_str().filter(|h| !h.is_empty())?;
    let port = parsed.port().map(|p| format!(":{p}")).unwrap_or_default();
    Some(format!("{}://{}{}", parsed.scheme(), host, port))
}

/// Ask a URL whether it is a Gameyfin server, and whether we are already signed in.
/// `getUserInfo` is `@AnonymousAllowed`, so one call serves as both liveness and session check.
pub async fn probe(http: &reqwest::Client, url: &str) -> ServerProbe {
    let Some(normalized) = normalize_url(url) else {
        return ServerProbe {
            url: url.to_string(),
            reachable: false,
            authenticated: false,
            message: Some("That does not look like a valid address.".into()),
        };
    };

    let endpoint = format!("{normalized}/connect/UserEndpoint/getUserInfo");
    let response = http
        .post(&endpoint)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::ACCEPT, "application/json")
        .body("{}")
        .send()
        .await;

    match response {
        Ok(resp) => {
            let status = resp.status();
            // 401/403 still proves this is Gameyfin: the endpoint exists and is refusing an
            // anonymous caller, which is what signing in fixes.
            if status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                let authenticated = !body.trim().is_empty() && body.trim() != "null";
                ServerProbe {
                    url: normalized,
                    reachable: true,
                    authenticated,
                    message: None,
                }
            } else if status.as_u16() == 401 || status.as_u16() == 403 {
                ServerProbe {
                    url: normalized,
                    reachable: true,
                    authenticated: false,
                    message: None,
                }
            } else {
                ServerProbe {
                    url: normalized,
                    reachable: false,
                    authenticated: false,
                    message: Some(format!(
                        "The server answered with HTTP {status}. Is this a Gameyfin instance?"
                    )),
                }
            }
        }
        Err(e) => ServerProbe {
            url: normalized,
            reachable: false,
            authenticated: false,
            message: Some(friendly_transport_error(&e)),
        },
    }
}

/// Turn a transport failure into something a person can act on.
fn friendly_transport_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "The server did not respond in time.".into()
    } else if e.is_connect() {
        "Could not connect. Check the address, and that the server is running.".into()
    } else if e.is_request() {
        "That address could not be requested.".into()
    } else {
        format!("Could not reach the server: {e}")
    }
}

/// A mainstream desktop browser user agent: several identity providers (Google especially)
/// refuse logins from anything they read as an embedded webview.
#[cfg(target_os = "linux")]
const LOGIN_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";
#[cfg(target_os = "windows")]
const LOGIN_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:128.0) Gecko/20100101 Firefox/128.0";
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
const LOGIN_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:128.0) Gecko/20100101 Firefox/128.0";

/// Progress of the sign-in window, forwarded to the wizard so it can show what is happening.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginProgress {
    pub host: String,
    /// True once the window is back on the Gameyfin origin.
    pub on_server: bool,
}

/// Open the sign-in window on the server's login page, configured to behave like an ordinary
/// browser with no navigation restriction (an SSO login can cross several origins).
pub fn open_login_window(app: &AppHandle, base_url: &str, direct: bool) -> Result<(), String> {
    // Re-opening should focus the existing window rather than stacking another.
    if let Some(existing) = app.get_webview_window(LOGIN_WINDOW) {
        let _ = existing.set_focus();
        return Ok(());
    }

    // `/loginredirect` is the server's own entry point: it forwards to the SSO provider when
    // configured, else the password form. `?direct=1` demands the password form regardless,
    // the escape hatch for a local account on a server that also has SSO.
    let path = if direct {
        "/loginredirect?direct=1"
    } else {
        "/loginredirect"
    };

    let url = url::Url::parse(&format!("{base_url}{path}"))
        .map_err(|e| format!("could not build the sign-in URL: {e}"))?;

    let server_host = url::Url::parse(base_url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default();

    let mut builder = WebviewWindowBuilder::new(app, LOGIN_WINDOW, WebviewUrl::External(url))
        .title("Sign in to Gameyfin")
        .inner_size(980.0, 800.0)
        .center()
        .focused(true)
        .user_agent(LOGIN_USER_AGENT);

    // Persist the login profile (separate from the main app's store) so "remember this
    // device" can be honoured.
    if let Ok(dir) = app.path().app_data_dir() {
        builder = builder.data_directory(dir.join("login-profile"));
    }

    let handle = app.clone();
    let host_for_nav = server_host.clone();
    builder = builder.on_navigation(move |url| {
        let host = url.host_str().unwrap_or_default().to_string();
        // Logged so a failed sign-in shows which host the flow stopped at.
        tracing::info!("sign-in window navigating to {url}");
        let _ = handle.emit(
            "login-progress",
            LoginProgress {
                on_server: !host.is_empty() && host == host_for_nav,
                host,
            },
        );
        // Never block: any origin may legitimately appear in an SSO chain.
        true
    });

    builder
        .build()
        .map_err(|e| format!("could not open the sign-in window: {e}"))?;

    Ok(())
}

/// Read the cookies the sign-in window holds for the Gameyfin origin.
pub fn harvest_cookies(app: &AppHandle, base_url: &str) -> HashMap<String, String> {
    let Some(window) = app.get_webview_window(LOGIN_WINDOW) else {
        return HashMap::new();
    };
    let Ok(url) = url::Url::parse(base_url) else {
        return HashMap::new();
    };

    match window.cookies_for_url(url) {
        Ok(cookies) => {
            let harvested: HashMap<String, String> = cookies
                .into_iter()
                .map(|c| (c.name().to_string(), c.value().to_string()))
                .collect();
            // Names only; the values are session secrets.
            tracing::debug!(
                names = ?harvested.keys().collect::<Vec<_>>(),
                "harvested cookies from the sign-in window"
            );
            harvested
        }
        Err(e) => {
            tracing::debug!("could not read cookies from the sign-in window: {e}");
            HashMap::new()
        }
    }
}

/// Whether the sign-in window is currently open.
pub fn login_window_open(app: &AppHandle) -> bool {
    app.get_webview_window(LOGIN_WINDOW).is_some()
}

/// Delete the sign-in window's stored profile: the escape hatch when a persisted provider
/// session gets into a state the user cannot clear from inside the window.
pub async fn reset_login_profile(app: &AppHandle) -> Result<(), String> {
    close_login_window(app);
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("could not locate the app data directory: {e}"))?
        .join("login-profile");

    match tokio::fs::remove_dir_all(&dir).await {
        Ok(()) => Ok(()),
        // Nothing to clear is a success.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("could not clear the sign-in profile: {e}")),
    }
}

pub fn close_login_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LOGIN_WINDOW) {
        let _ = window.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_a_scheme_when_one_is_missing() {
        assert_eq!(
            normalize_url("games.example.com").as_deref(),
            Some("https://games.example.com")
        );
    }

    #[test]
    fn keeps_an_explicit_scheme_and_port() {
        assert_eq!(
            normalize_url("http://192.168.1.10:8080").as_deref(),
            Some("http://192.168.1.10:8080")
        );
    }

    #[test]
    fn strips_paths_and_trailing_slashes() {
        assert_eq!(
            normalize_url("https://games.example.com/game/12").as_deref(),
            Some("https://games.example.com")
        );
        assert_eq!(
            normalize_url("https://games.example.com/").as_deref(),
            Some("https://games.example.com")
        );
    }

    #[test]
    fn rejects_nonsense() {
        assert_eq!(normalize_url(""), None);
        assert_eq!(normalize_url("   "), None);
        assert_eq!(normalize_url("ftp://games.example.com"), None);
    }

    #[test]
    fn rejects_a_scheme_with_no_host() {
        assert_eq!(normalize_url("https://"), None);
        assert_eq!(normalize_url("http://"), None);
        assert_eq!(normalize_url("https:///"), None);
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(
            normalize_url("  https://games.example.com  ").as_deref(),
            Some("https://games.example.com")
        );
    }
}
