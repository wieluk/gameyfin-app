//! Sign-in. Gameyfin has no token endpoint, so a real webview logs in and the session
//! cookies are read back out; SSO then needs no handling of its own.

use std::collections::HashMap;

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

/// Label of the sign-in window, so it can be found and closed again.
pub const LOGIN_WINDOW: &str = "gameyfin-login";

#[derive(Debug, Clone, PartialEq, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
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

/// Reduces a bare host, a full URL or a pasted link to a base origin.
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
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LoginProgress {
    pub host: String,
    /// True once the window is back on the Gameyfin origin.
    pub on_server: bool,
}

/// Opens the sign-in window, with no navigation restriction: an SSO login crosses origins.
pub fn open_login_window(app: &AppHandle, base_url: &str, direct: bool) -> Result<(), String> {
    // Re-opening should focus the existing window rather than stacking another.
    if let Some(existing) = app.get_webview_window(LOGIN_WINDOW) {
        let _ = existing.set_focus();
        return Ok(());
    }

    // `/loginredirect` forwards to SSO when configured, else the password form. `?direct=1` forces
    // the form, for a local account on a server that also has SSO.
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
        // Host and path only: the query carries the authorization code and session tokens.
        tracing::info!(%host, path = url.path(), "sign-in window navigating");
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

pub fn login_window_open(app: &AppHandle) -> bool {
    app.get_webview_window(LOGIN_WINDOW).is_some()
}

/// Deletes the stored profile, which is also what stops the next sign-in reusing a session.
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
    fn normalize_url_reduces_an_address_to_scheme_and_host() {
        for (input, expected) in [
            ("games.example.com", Some("https://games.example.com")),
            // An explicit scheme and port are kept as typed.
            ("http://192.168.1.10:8080", Some("http://192.168.1.10:8080")),
            (
                "https://games.example.com/game/12",
                Some("https://games.example.com"),
            ),
            (
                "https://games.example.com/",
                Some("https://games.example.com"),
            ),
            (
                "  https://games.example.com  ",
                Some("https://games.example.com"),
            ),
            ("", None),
            ("   ", None),
            ("ftp://games.example.com", None),
            // A scheme with no host is not an address.
            ("https://", None),
            ("http://", None),
            ("https:///", None),
        ] {
            assert_eq!(normalize_url(input).as_deref(), expected, "{input:?}");
        }
    }
}
