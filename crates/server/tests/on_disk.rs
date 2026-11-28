//! What is left in the database files once a message has been delivered or has expired.

mod common;

use std::path::Path;

use axum::http::StatusCode;
use common::*;
use deaddrop_core::encoding::{b64, random_id};
use deaddrop_core::soft::SoftDevice;
use deaddropd::{AppState, Store};
use serde_json::json;

/// Whether `needle` appears anywhere in any file in `directory` (the database, its
/// write-ahead log and anything else SQLite leaves there).
fn on_disk(directory: &Path, needle: &[u8]) -> bool {
    std::fs::read_dir(directory).unwrap().any(|entry| {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        bytes.windows(needle.len()).any(|window| window == needle)
    })
}

/// A server on a real database file with an owner and one member in a channel.
async fn file_server(directory: &Path, max_age: u64) -> (Server, Phone, Phone, String) {
    let store = Store::open(&directory.join("deaddrop.db")).unwrap();
    let (state, operator_token) = AppState::open(store).await.unwrap();
    let router = deaddropd::router(state.clone());
    let device = SoftDevice::new();
    let mut body = registration(&device);
    body["token"] = json!(operator_token.unwrap());
    assert_eq!(call(&router, POST, "/v1/devices", None, Some(body)).await.0, StatusCode::CREATED);
    let token = sign_in(&router, &state, &device).await.unwrap();
    let server = Server { state, router, operator: Phone { id: device.device_id(), device, token } };
    let (owner, member) = (server.phone().await, server.phone().await);
    let channel = server.channel_with(&owner, max_age, &[(&member, "Member")]).await;
    (server, owner, member, channel)
}

/// A body that could not be in the files by accident.
fn marker() -> Vec<u8> {
    format!("MARKER-{}-{}", random_id(), random_id()).into_bytes()
}

#[tokio::test]
async fn a_delivered_message_leaves_no_bytes_in_the_database_files() {
    let scratch = Scratch::new();
    let (server, owner, member, channel) = file_server(&scratch.0, 3600).await;
    let body = marker();
    let path = format!("/v1/channels/{channel}/messages");
    let (status, _) = server.as_phone(&owner, POST, &path, Some(json!({ "body": b64(&body) }))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(on_disk(&scratch.0, &body), "while queued, the sealed body is in the files (the test can see it)");

    server.collect(&member).await;
    assert!(!on_disk(&scratch.0, &body), "the moment it is delivered, not one copy of the body remains");
}

#[tokio::test]
async fn an_expired_message_leaves_no_bytes_in_the_database_files() {
    let scratch = Scratch::new();
    let (server, owner, _member, channel) = file_server(&scratch.0, 60).await;
    let body = marker();
    let path = format!("/v1/channels/{channel}/messages");
    server.as_phone(&owner, POST, &path, Some(json!({ "body": b64(&body) }))).await;
    assert!(on_disk(&scratch.0, &body));

    server.state.clock.advance(61);
    assert_eq!(deaddropd::sweep(&server.state).await.unwrap(), 1);
    assert!(!on_disk(&scratch.0, &body), "past the maximum message age, not one copy of the body remains");
}

#[tokio::test]
async fn the_database_never_holds_a_token_in_the_clear() {
    let scratch = Scratch::new();
    let (server, owner, _member, channel) = file_server(&scratch.0, 3600).await;
    let server_invite = server.invite(5, 3600).await;
    let (_, reply) = server.as_phone(&owner, POST, &format!("/v1/channels/{channel}/invites"), Some(json!({}))).await;
    let channel_invite = reply["token"].as_str().unwrap().to_owned();
    deaddropd::sweep(&server.state).await.unwrap();
    for secret in [&server_invite, &channel_invite, &owner.token, &server.operator.token] {
        assert!(!on_disk(&scratch.0, secret.as_bytes()), "only hashes of tokens are stored");
    }
}

#[tokio::test]
async fn removing_the_last_recipient_wipes_the_bytes_too() {
    let scratch = Scratch::new();
    let (server, owner, _member, channel) = file_server(&scratch.0, 3600).await;
    let body = marker();
    let path = format!("/v1/channels/{channel}/messages");
    server.as_phone(&owner, POST, &path, Some(json!({ "body": b64(&body) }))).await;
    assert!(on_disk(&scratch.0, &body));

    let alone = roster(&channel, 3, &[(&owner, "Owner")]);
    assert_eq!(server.put_roster(&owner, &channel, &alone).await.0, StatusCode::NO_CONTENT);
    assert!(!on_disk(&scratch.0, &body));
}

#[tokio::test]
async fn a_database_from_before_join_requests_were_stamped_is_brought_up_to_date() {
    let scratch = Scratch::new();
    let path = scratch.0.join("old.db");
    {
        let old = rusqlite::Connection::open(&path).unwrap();
        old.execute_batch(
            "CREATE TABLE join_requests (
                 channel TEXT NOT NULL, device TEXT NOT NULL, name TEXT NOT NULL,
                 PRIMARY KEY (channel, device)
             ) WITHOUT ROWID;
             INSERT INTO join_requests VALUES ('a channel', 'a device', 'Old');",
        )
        .unwrap();
    }
    for _ in 0..2 {
        // Opened twice: the change is made once and is harmless to repeat.
        let store = Store::open(&path).unwrap();
        let kept: (String, String) = store
            .run(|db| {
                Ok(db.query_row("SELECT name, stamp FROM join_requests", [], |row| Ok((row.get(0)?, row.get(1)?)))?)
            })
            .await
            .unwrap();
        assert_eq!(kept, ("Old".to_owned(), String::new()), "kept, with no stamp, for the owner's phone to decline");
    }
}
