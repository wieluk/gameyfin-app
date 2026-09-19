//! A reverse proxy in front of Gameyfin, such as Pangolin, authenticates separately. When its
//! session expires it answers instead of Gameyfin, which must read as "sign in again" rather
//! than as a server that is down.

use std::sync::Arc;

use gameyfin_api::{ApiError, DeviceTokenAuth, GameyfinClient};

fn client(url: &str) -> GameyfinClient {
    GameyfinClient::new(url, Arc::new(DeviceTokenAuth::new("token"))).unwrap()
}

#[tokio::test]
async fn a_redirect_to_the_proxy_portal_asks_for_a_sign_in() {
    let mut portal = mockito::Server::new_async().await;
    portal
        .mock("POST", "/auth/resource/1")
        .with_header("content-type", "text/html")
        .with_body("<html>Sign in</html>")
        .create_async()
        .await;

    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/connect/UserEndpoint/getUserInfo")
        .with_status(302)
        .with_header("location", &format!("{}/auth/resource/1", portal.url()))
        .create_async()
        .await;

    let error = client(&server.url()).user_info().await.unwrap_err();

    assert!(
        matches!(error, ApiError::ProxyAuthRequired { .. }),
        "expected a proxy sign-in, got {error:?}"
    );
    assert!(error.is_auth(), "it is fixed by signing in again");
    assert!(!error.is_unreachable(), "the server is not down");
}

#[tokio::test]
async fn a_portal_served_in_place_of_json_asks_for_a_sign_in() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/connect/UserEndpoint/getUserInfo")
        .with_header("content-type", "text/html; charset=utf-8")
        .with_body("<!DOCTYPE html><html>Sign in</html>")
        .create_async()
        .await;

    let error = client(&server.url()).user_info().await.unwrap_err();

    assert!(
        matches!(error, ApiError::ProxyAuthRequired { .. }),
        "expected a proxy sign-in, got {error:?}"
    );
}

#[tokio::test]
async fn gameyfins_own_html_error_page_is_not_mistaken_for_a_portal() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/connect/UserEndpoint/getUserInfo")
        .with_status(500)
        .with_header("content-type", "text/html")
        .with_body("<html>Whitelabel Error Page</html>")
        .create_async()
        .await;

    let error = client(&server.url()).user_info().await.unwrap_err();

    assert!(
        !matches!(error, ApiError::ProxyAuthRequired { .. }),
        "a server error is not a sign-in prompt, got {error:?}"
    );
}
