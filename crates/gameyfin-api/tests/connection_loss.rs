//! Losing the connection must never look like being signed out: the wizard that follows
//! offers to switch servers, discarding the session and cached library.

use std::sync::Arc;

use gameyfin_api::{DeviceTokenAuth, GameyfinClient};

fn client(url: &str) -> GameyfinClient {
    GameyfinClient::new(url, Arc::new(DeviceTokenAuth::new("token"))).unwrap()
}

#[tokio::test]
async fn a_body_cut_short_is_a_transport_failure_not_an_empty_answer() {
    // A server that dies after its headers: Content-Length promises more than it sends.
    // Read as an empty body this deserializes to null, which reads as "nobody is signed in".
    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/connect/UserEndpoint/getUserInfo")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_header("content-length", "4096")
        .with_body("{\"id\":1,")
        .create_async()
        .await;

    let error = client(&server.url()).user_info().await.unwrap_err();

    assert!(
        error.is_unreachable(),
        "a truncated body should read as unreachable, got {error:?}"
    );
    assert!(
        !error.is_auth(),
        "a truncated body must never read as a refused session, got {error:?}"
    );
}

#[tokio::test]
async fn a_restarting_proxy_is_unreachable() {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/connect/UserEndpoint/getUserInfo")
        .with_status(503)
        .with_body("<html><body>Service Unavailable</body></html>")
        .create_async()
        .await;

    let error = client(&server.url()).user_info().await.unwrap_err();

    assert!(error.is_unreachable(), "got {error:?}");
    assert!(!error.is_auth(), "got {error:?}");
}

#[tokio::test]
async fn a_genuinely_signed_out_session_is_still_reported_as_such() {
    // A real sign-out must still read as one, or the user is never asked to sign in again.
    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/connect/UserEndpoint/getUserInfo")
        .with_status(401)
        .create_async()
        .await;

    let error = client(&server.url()).user_info().await.unwrap_err();

    assert!(error.is_auth(), "got {error:?}");
    assert!(!error.is_unreachable(), "got {error:?}");
}

#[tokio::test]
async fn an_empty_body_from_a_healthy_server_still_means_nobody_is_signed_in() {
    // Hilla answers a void or null result with an empty body, and that has to keep working.
    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/connect/UserEndpoint/getUserInfo")
        .with_status(200)
        .with_body("")
        .create_async()
        .await;

    let user = client(&server.url()).user_info().await.unwrap();

    assert!(user.is_none());
}
