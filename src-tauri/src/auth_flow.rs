//! First-run connection and sign-in.
//!
//! Gameyfin authenticates browsers with a session cookie and offers no token endpoint, so
//! the only way in today is to *be* a browser for the duration of the login. A dedicated
//! webview window is opened on the server's login page; the user completes whatever the
//! server asks, a local password, or a full redirect to an OIDC provider such as
//! Authentik, and the resulting cookies are read back out of that window.
//!
//! Doing it this way means SSO needs no special handling at all: any provider, any number
//! of redirects, MFA included, all happen inside a real browser engine. We only look at
//! the end state, which is "does the Gameyfin origin now have cookies that authenticate".

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

/// Normalise user input into a base URL.
///
/// People type `games.example.com`, `https://games.example.com/`, or paste a deep link.
/// All of those should work rather than producing an obscure failure later.
pub fn normalize_url(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    // The scheme must be detected on the raw input. Stripping a trailing slash first
    // turns "https://" into "https:", which no longer looks like it carries a scheme and
    // would then be treated as the *hostname* of an invented https:// URL.
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        // Default to HTTPS: a self-hosted server on the open internet should not be
        // downgraded silently, and http:// still works if typed explicitly.
        format!("https://{trimmed}")
    };

    let parsed = url::Url::parse(&with_scheme).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }

    // Rebuilding from the parsed parts drops any path, query or trailing slash, so the
    // result is a bare origin whatever was pasted in.
    let host = parsed.host_str().filter(|h| !h.is_empty())?;
    let port = parsed.port().map(|p| format!(":{p}")).unwrap_or_default();
    Some(format!("{}://{}{}", parsed.scheme(), host, port))
}

/// Ask a URL whether it is a Gameyfin server, and whether we are already signed in.
///
/// `UserEndpoint.getUserInfo` is `@AnonymousAllowed`, so it answers for an anonymous
/// caller instead of rejecting, which makes it both a liveness probe and a session
/// check in one call.
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
            // 401/403 still proves this is Gameyfin, it means the endpoint exists and
            // is refusing an anonymous caller, which is a correct answer for a private
            // instance and exactly what signing in will fix.
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

/// A user agent that identity providers recognise.
///
/// WebKitGTK and WebView2 announce themselves with their own product tokens, and several
/// identity providers, Google most famously, but others follow suit, refuse logins from
/// anything they read as an embedded webview. Presenting a mainstream desktop browser
/// string avoids being turned away for the wrong reason.
#[cfg(target_os = "linux")]
const LOGIN_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";
#[cfg(target_os = "windows")]
const LOGIN_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:128.0) Gecko/20100101 Firefox/128.0";
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
const LOGIN_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:128.0) Gecko/20100101 Firefox/128.0";

/// Progress of the sign-in window, forwarded to the wizard so it can show what is
/// happening instead of an opaque spinner.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginProgress {
    /// Host currently loaded, e.g. the identity provider's domain.
    pub host: String,
    /// True once the window is back on the Gameyfin origin.
    pub on_server: bool,
}

/// Open the sign-in window on the server's login page.
///
/// The window is configured to behave like an ordinary browser: a mainstream user agent,
/// a persistent profile directory so the provider's own session survives, and no
/// restriction on where it may navigate. That last point matters, a login can cross
/// several origins (the server, an identity provider, sometimes a proxy in front of both)
/// before returning, and blocking any of them breaks the flow.
pub fn open_login_window(app: &AppHandle, base_url: &str, direct: bool) -> Result<(), String> {
    // Re-opening should focus the existing window rather than stacking another.
    if let Some(existing) = app.get_webview_window(LOGIN_WINDOW) {
        let _ = existing.set_focus();
        return Ok(());
    }

    // `/loginredirect` is the server's own entry point: when SSO is configured it
    // forwards to `/oauth2/authorization/oidc`, and otherwise to the password form
    // (`core/security/LoginRedirectController.kt`). Opening `/login` directly, as this
    // did originally, lands on the password form even on an SSO-only instance, because
    // that page renders no identity-provider button of its own.
    //
    // `?direct=1` is the documented way to demand the password form regardless, which is
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

    // Keep the login profile out of the main app's store, but persist it: an identity
    // provider that offers "remember this device" should be able to honour it.
    if let Ok(dir) = app.path().app_data_dir() {
        builder = builder.data_directory(dir.join("login-profile"));
    }

    let handle = app.clone();
    let host_for_nav = server_host.clone();
    builder = builder.on_navigation(move |url| {
        let host = url.host_str().unwrap_or_default().to_string();
        // Logged because a failed sign-in is otherwise invisible: this shows exactly
        // which host the flow stopped at.
        tracing::info!("sign-in window navigating to {url}");
        let _ = handle.emit(
            "login-progress",
            LoginProgress {
                on_server: !host.is_empty() && host == host_for_nav,
                host,
            },
        );
        // Never block. Any origin may legitimately appear in an SSO chain.
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
            // Names only, the values are session secrets. Which cookies arrived is
            // exactly what is needed to tell "the provider set its own" from "the server
            // issued a session".
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

/// Delete the sign-in window's stored profile.
///
/// A provider session can get into a state the user cannot clear from inside the window,
/// a half-finished flow, a stale cookie, and with the profile persisted that survives
/// restarts. This is the escape hatch.
pub async fn reset_login_profile(app: &AppHandle) -> Result<(), String> {
    close_login_window(app);
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("could not locate the app data directory: {e}"))?
        .join("login-profile");

    match tokio::fs::remove_dir_all(&dir).await {
        Ok(()) => Ok(()),
        // Nothing to clear is a success, not a failure.
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
        // Pasting a deep link from the web UI should still configure the server.
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
        // Stripping the trailing slash before looking for a scheme used to turn these
        // into a URL whose *host* was "https".
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
