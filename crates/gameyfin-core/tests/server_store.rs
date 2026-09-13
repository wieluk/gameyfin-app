//! The server store against a real HTTP server, for the answers other stores give from disk.

use std::sync::Arc;

use gameyfin_api::{DeviceTokenAuth, GameyfinClient};
use gameyfin_core::save_store::{SaveStore, ServerStore};

fn save_json(id: i64, game_id: i64) -> String {
    format!(
        r#"{{"id":{id},"gameId":{game_id},"sizeBytes":1,"contentHash":"h{id}",
            "platform":"WINDOWS","locked":false,"createdAt":"2026-01-01T00:00:00Z"}}"#
    )
}

#[tokio::test]
async fn names_each_game_with_saves_once() {
    let mut server = mockito::Server::new_async().await;
    let mock = server
        .mock("POST", "/connect/SaveSyncEndpoint/getMySaves")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(format!(
            "[{},{},{}]",
            save_json(3, 42),
            save_json(2, 7),
            save_json(1, 42)
        ))
        .create_async()
        .await;
    let client =
        GameyfinClient::new(server.url(), Arc::new(DeviceTokenAuth::new("token"))).unwrap();

    // A migration copies exactly these games.
    assert_eq!(vec![7, 42], ServerStore::new(client).games().await.unwrap());
    mock.assert_async().await;
}
