//! The server version is optional twice over: the endpoint may be missing, and a server built
//! without build info reports none.

use std::sync::Arc;

use gameyfin_api::{DeviceTokenAuth, GameyfinClient};

fn client(url: &str) -> GameyfinClient {
    GameyfinClient::new(url, Arc::new(DeviceTokenAuth::new("token"))).unwrap()
}

async fn version(status: usize, body: &str) -> Option<String> {
    let mut server = mockito::Server::new_async().await;
    server
        .mock("POST", "/connect/VersionEndpoint/getVersion")
        .with_status(status)
        .with_body(body)
        .create_async()
        .await;
    client(&server.url()).server_version().await.ok().flatten()
}

#[tokio::test]
async fn reads_the_version_a_server_reports() {
    assert_eq!(version(200, "\"2.4.0\"").await, Some("2.4.0".to_string()));
}

#[tokio::test]
async fn a_server_without_one_reports_nothing() {
    assert_eq!(version(200, "null").await, None);
    // Servers without the endpoint refuse it, which must not read as an error worth showing.
    assert_eq!(version(403, "").await, None);
}
