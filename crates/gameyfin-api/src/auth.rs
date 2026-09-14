//! Gameyfin 2.4 has no token auth, so [`CookieSessionAuth`] reuses a webview's session cookies
//! and derives CSRF like Hilla's client. [`DeviceTokenAuth`] waits on server support.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use reqwest::RequestBuilder;
use tokio::sync::RwLock;

use crate::error::{ApiError, ApiResult};

#[async_trait]
pub trait AuthStrategy: Send + Sync + std::fmt::Debug {
    async fn apply(&self, req: RequestBuilder) -> ApiResult<RequestBuilder>;

    /// Called after a 401 or 403. `true` when something was refreshed and one retry is worth it.
    async fn on_rejected(&self) -> ApiResult<bool> {
        Ok(false)
    }

    fn describe(&self) -> &'static str;
}

/// Bearer-token auth against the device-token endpoints.
#[derive(Debug, Clone)]
pub struct DeviceTokenAuth {
    token: Arc<RwLock<String>>,
}

impl DeviceTokenAuth {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: Arc::new(RwLock::new(token.into())),
        }
    }
}

#[async_trait]
impl AuthStrategy for DeviceTokenAuth {
    async fn apply(&self, req: RequestBuilder) -> ApiResult<RequestBuilder> {
        let token = self.token.read().await;
        Ok(req.bearer_auth(token.as_str()))
    }

    fn describe(&self) -> &'static str {
        "device-token"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CsrfToken {
    pub header: String,
    pub value: String,
}

const SPRING_CSRF_COOKIE: &str = "XSRF-TOKEN";
const SPRING_CSRF_HEADER: &str = "X-XSRF-TOKEN";
/// Vaadin's names, used when Spring's are absent.
const VAADIN_CSRF_COOKIE: &str = "csrfToken";
const VAADIN_CSRF_HEADER: &str = "X-CSRF-Token";

/// CSRF details scraped from the served page. Spring's two repositories put the token in
/// different places, so the header name and the token are looked up independently.
#[derive(Debug, Default, Clone)]
struct PageCsrf {
    spring_header: Option<String>,
    spring_token: Option<String>,
    vaadin_token: Option<String>,
}

/// Session-cookie auth, mirroring what a logged-in browser sends.
#[derive(Debug)]
pub struct CookieSessionAuth {
    base_url: String,
    http: reqwest::Client,
    cookies: RwLock<HashMap<String, String>>,
    /// Cached page scrape. `Some(_)` even when empty, so the index page is fetched at most
    /// once per reset rather than once per call.
    page_csrf: RwLock<Option<PageCsrf>>,
}

impl CookieSessionAuth {
    pub fn new(base_url: impl Into<String>, http: reqwest::Client) -> Self {
        // A client that never follows redirects, so the CSRF probe cannot be led into
        // starting an authentication flow.
        let http = reqwest::Client::builder()
            .user_agent(concat!("Gameyfin-App/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or(http);

        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            http,
            cookies: RwLock::new(HashMap::new()),
            page_csrf: RwLock::new(None),
        }
    }

    /// Replace the cookie jar with cookies harvested from the login webview.
    pub async fn set_cookies(&self, cookies: HashMap<String, String>) {
        *self.cookies.write().await = cookies;
        // Cookies changed, so any cached page token is likely stale too.
        *self.page_csrf.write().await = None;
    }

    pub async fn cookies(&self) -> HashMap<String, String> {
        self.cookies.read().await.clone()
    }

    /// Forget the cached CSRF scrape. Call after a rejected request or a URL change.
    pub async fn reset_csrf(&self) {
        *self.page_csrf.write().await = None;
    }

    async fn page_csrf(&self) -> PageCsrf {
        if let Some(cached) = self.page_csrf.read().await.clone() {
            return cached;
        }

        let mut info = PageCsrf::default();
        let cookies = self.cookies.read().await.clone();

        // `/login`, not `/`: on an SSO server a GET to `/` restarts the authorization the user is
        // part-way through. Redirects are not followed for the same reason.
        let req = self
            .http
            .get(format!("{}/login", self.base_url))
            .header(reqwest::header::COOKIE, cookie_header(&cookies));

        match req.send().await {
            Ok(resp) => match resp.text().await {
                Ok(body) => info = parse_page_csrf(&body),
                Err(e) => tracing::debug!("could not read index page body for CSRF: {e}"),
            },
            Err(e) => tracing::debug!("could not fetch index page for CSRF: {e}"),
        }

        *self.page_csrf.write().await = Some(info.clone());
        info
    }

    /// Follows Hilla's resolution order: a Spring token wins over Vaadin's.
    pub async fn csrf_token(&self) -> Option<CsrfToken> {
        let cookies = self.cookies.read().await.clone();
        let page = self.page_csrf().await;

        let spring = cookies
            .get(SPRING_CSRF_COOKIE)
            .cloned()
            .or(page.spring_token.clone());
        if let Some(value) = spring {
            return Some(CsrfToken {
                header: page
                    .spring_header
                    .unwrap_or_else(|| SPRING_CSRF_HEADER.to_string()),
                value,
            });
        }

        let vaadin = cookies
            .get(VAADIN_CSRF_COOKIE)
            .cloned()
            .or(page.vaadin_token);
        vaadin.map(|value| CsrfToken {
            header: VAADIN_CSRF_HEADER.to_string(),
            value,
        })
    }
}

pub fn cookie_header(cookies: &HashMap<String, String>) -> String {
    cookies
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

#[async_trait]
impl AuthStrategy for CookieSessionAuth {
    async fn apply(&self, req: RequestBuilder) -> ApiResult<RequestBuilder> {
        let cookies = self.cookies.read().await.clone();
        if cookies.is_empty() {
            return Err(ApiError::Unauthenticated(
                "no session cookies; log in first".into(),
            ));
        }

        let mut req = req.header(reqwest::header::COOKIE, cookie_header(&cookies));
        if let Some(csrf) = self.csrf_token().await {
            req = req.header(csrf.header, csrf.value);
        }
        Ok(req)
    }

    async fn on_rejected(&self) -> ApiResult<bool> {
        // A rejected call is usually a stale CSRF token, which is worth re-reading once
        // before reporting the session as dead.
        self.reset_csrf().await;
        Ok(true)
    }

    fn describe(&self) -> &'static str {
        "cookie-session"
    }
}

fn parse_page_csrf(body: &str) -> PageCsrf {
    // Compiled per call rather than cached: this runs at most once per session reset.
    let header_re =
        Regex::new(r#"(?i)<meta\s+name=["']_csrf_header["']\s+content=["']([^"']+)["']"#);
    let token_re = Regex::new(r#"(?i)<meta\s+name=["']_csrf["']\s+content=["']([^"']+)["']"#);
    let vaadin_re = Regex::new(r#"["']csrfToken["']\s*:\s*["']([^"']+)["']"#);

    let first_group = |re: Result<Regex, regex::Error>| -> Option<String> {
        re.ok()?
            .captures(body)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string())
    };

    PageCsrf {
        spring_header: first_group(header_re),
        spring_token: first_group(token_re),
        vaadin_token: first_group(vaadin_re),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spring_meta_tags() {
        let body = r#"<html><head>
            <meta name="_csrf_header" content="X-Custom-Csrf"/>
            <meta name="_csrf" content="tok-123"/>
        </head></html>"#;
        let csrf = parse_page_csrf(body);
        assert_eq!(csrf.spring_header.as_deref(), Some("X-Custom-Csrf"));
        assert_eq!(csrf.spring_token.as_deref(), Some("tok-123"));
        assert_eq!(csrf.vaadin_token, None);
    }

    #[test]
    fn parses_vaadin_inline_token() {
        let body = r#"<script>window.Vaadin = {TypeScript: {"csrfToken":"vaadin-abc"}};</script>"#;
        let csrf = parse_page_csrf(body);
        assert_eq!(csrf.vaadin_token.as_deref(), Some("vaadin-abc"));
        assert_eq!(csrf.spring_token, None);
    }

    #[test]
    fn empty_page_yields_nothing() {
        let csrf = parse_page_csrf("<html></html>");
        assert!(csrf.spring_header.is_none());
        assert!(csrf.spring_token.is_none());
        assert!(csrf.vaadin_token.is_none());
    }

    #[tokio::test]
    async fn spring_cookie_beats_vaadin_cookie() {
        let auth = CookieSessionAuth::new("http://localhost:8080", reqwest::Client::new());
        auth.set_cookies(HashMap::from([
            ("XSRF-TOKEN".to_string(), "spring-tok".to_string()),
            ("csrfToken".to_string(), "vaadin-tok".to_string()),
            ("JSESSIONID".to_string(), "sess".to_string()),
        ]))
        .await;
        // Index page is unreachable here, so resolution falls back to cookies alone.
        let csrf = auth.csrf_token().await.expect("a token");
        assert_eq!(csrf.header, "X-XSRF-TOKEN");
        assert_eq!(csrf.value, "spring-tok");
    }

    #[tokio::test]
    async fn falls_back_to_vaadin_cookie() {
        let auth = CookieSessionAuth::new("http://localhost:8080", reqwest::Client::new());
        auth.set_cookies(HashMap::from([(
            "csrfToken".to_string(),
            "vaadin-tok".to_string(),
        )]))
        .await;
        let csrf = auth.csrf_token().await.expect("a token");
        assert_eq!(csrf.header, "X-CSRF-Token");
        assert_eq!(csrf.value, "vaadin-tok");
    }

    #[tokio::test]
    async fn refuses_to_sign_without_cookies() {
        let auth = CookieSessionAuth::new("http://localhost:8080", reqwest::Client::new());
        let req = reqwest::Client::new().get("http://localhost:8080/connect/X/y");
        let err = auth.apply(req).await.unwrap_err();
        assert!(err.is_auth(), "expected an auth error, got {err:?}");
    }
}
