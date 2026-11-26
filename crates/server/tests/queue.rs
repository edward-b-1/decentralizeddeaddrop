//! The delivery queue: a message is wiped when everyone has it, or when it is too old.

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use common::*;
use deaddrop_core::encoding::b64;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

struct Trio {
    server: Server,
    a: Phone,
    b: Phone,
    c: Phone,
    channel: String,
}

/// A channel owned by `a` with members `b` and `c`.
async fn trio(max_age: u64) -> Trio {
    let server = Server::new().await;
    let (a, b, c) = (server.phone().await, server.phone().await, server.phone().await);
    let channel = server.channel_with(&a, max_age, &[(&b, "B"), (&c, "C")]).await;
    Trio { server, a, b, c, channel }
}

#[tokio::test]
async fn a_message_is_wiped_the_moment_everyone_has_it() {
    let Trio { server, a, b, c, channel } = trio(3600).await;
    let (status, sent) = server.send(&a, &channel, "hello").await;
    assert_eq!((status, &sent["recipients"]), (StatusCode::CREATED, &json!(2)));
    assert_eq!((server.rows("messages").await, server.rows("pending").await), (1, 2), "one copy, two marks");

    assert!(server.queue(&a).await.is_empty(), "the sender is not a recipient");
    let waiting = server.queue(&b).await;
    assert_eq!(waiting, vec![(sent["id"].as_str().unwrap().to_owned(), encoded("hello"))]);
    assert_eq!(server.queue(&b).await, waiting, "fetching alone does not remove anything");

    server.ack(&b, &[waiting[0].0.clone()]).await;
    assert!(server.queue(&b).await.is_empty());
    assert_eq!(server.rows("messages").await, 1, "still held for the member who has not collected it");

    assert_eq!(server.collect(&c).await, vec![encoded("hello")]);
    assert_eq!((server.rows("messages").await, server.rows("pending").await), (0, 0), "delivered to all: wiped");
}

#[tokio::test]
async fn acknowledging_for_someone_else_does_nothing() {
    let Trio { server, a, b, c, channel } = trio(3600).await;
    let (_, sent) = server.send(&a, &channel, "hello").await;
    let id = sent["id"].as_str().unwrap().to_owned();
    let outsider = server.phone().await;
    for phone in [&a, &outsider] {
        server.ack(phone, &[id.clone(), "no-such-message".into()]).await;
    }
    assert_eq!(server.rows("pending").await, 2);
    assert_eq!(server.queue(&b).await.len(), 1);
    assert_eq!(server.queue(&c).await.len(), 1);
}

#[tokio::test]
async fn a_message_past_the_channels_maximum_age_is_wiped_undelivered() {
    let Trio { server, a, b, c, channel } = trio(600).await;
    server.send(&a, &channel, "early").await;
    server.state.clock.advance(300);
    server.send(&a, &channel, "late").await;
    server.collect(&b).await;

    server.state.clock.advance(301);
    assert_eq!(server.queue(&c).await.len(), 1, "an expired message is never handed out, even before the sweep");
    assert_eq!(deaddropd::sweep(&server.state).await.unwrap(), 1);
    assert_eq!(server.rows("messages").await, 1);

    server.state.clock.advance(300);
    assert_eq!(deaddropd::sweep(&server.state).await.unwrap(), 1);
    assert_eq!((server.rows("messages").await, server.rows("pending").await), (0, 0));
    assert!(server.queue(&c).await.is_empty());
}

#[tokio::test]
async fn changing_the_maximum_age_applies_to_what_is_already_queued() {
    let Trio { server, a, b, channel, .. } = trio(86_400).await;
    server.send(&a, &channel, "hello").await;
    server.state.clock.advance(120);

    let body = json!({ "channel": a.device.seal(&channel_record(&a, &channel, 60, 2)) });
    let (status, _) = server.as_phone(&a, PUT, &format!("/v1/channels/{channel}"), Some(body)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(server.queue(&b).await.is_empty(), "already older than the new maximum");
    assert_eq!(deaddropd::sweep(&server.state).await.unwrap(), 1);
}

#[tokio::test]
async fn a_removed_member_gets_nothing_more_and_does_not_hold_messages_back() {
    let Trio { server, a, b, c, channel } = trio(3600).await;
    server.send(&a, &channel, "before").await;
    server.collect(&b).await;
    assert_eq!(server.rows("messages").await, 1, "waiting only for c");

    let without_c = roster(&channel, 3, &[(&a, "Owner"), (&b, "B")]);
    assert_eq!(server.put_roster(&a, &channel, &without_c).await.0, StatusCode::NO_CONTENT);
    assert_eq!(server.rows("messages").await, 0, "nobody left to deliver to: wiped");
    assert!(server.queue(&c).await.is_empty());

    let (_, sent) = server.send(&a, &channel, "after").await;
    assert_eq!(sent["recipients"], json!(1));
    assert!(server.queue(&c).await.is_empty());
    assert_eq!(server.send(&c, &channel, "still here?").await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn leaving_stops_delivery_both_ways() {
    let Trio { server, a, b, c, channel } = trio(3600).await;
    server.send(&a, &channel, "before").await;
    let (status, _) = server.as_phone(&c, DELETE, &format!("/v1/channels/{channel}/members/me"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(server.queue(&c).await.is_empty());
    assert_eq!(server.send(&c, &channel, "from outside").await.0, StatusCode::NOT_FOUND);
    assert_eq!(server.collect(&b).await.len(), 1);
    assert_eq!(server.rows("messages").await, 0);
}

#[tokio::test]
async fn a_deleted_device_does_not_hold_messages_back() {
    let Trio { server, a, b, c, channel } = trio(3600).await;
    server.send(&a, &channel, "hello").await;
    server.collect(&b).await;
    assert_eq!(server.as_phone(&c, DELETE, "/v1/devices/me", None).await.0, StatusCode::NO_CONTENT);
    assert_eq!((server.rows("messages").await, server.rows("pending").await), (0, 0));
}

#[tokio::test]
async fn a_message_can_be_addressed_to_some_members_only() {
    let Trio { server, a, b, c, channel } = trio(3600).await;
    let path = format!("/v1/channels/{channel}/messages");
    let outsider = server.phone().await;

    let body = json!({ "body": encoded("for b"), "to": [b.id] });
    let (status, sent) = server.as_phone(&a, POST, &path, Some(body)).await;
    assert_eq!((status, &sent["recipients"]), (StatusCode::CREATED, &json!(1)));
    assert!(server.queue(&c).await.is_empty());
    assert_eq!(server.collect(&b).await, vec![encoded("for b")]);

    for to in [json!([outsider.id]), json!([a.id]), json!([b.id, "nobody"])] {
        let body = json!({ "body": encoded("x"), "to": to });
        assert_eq!(server.as_phone(&a, POST, &path, Some(body)).await.0, StatusCode::BAD_REQUEST);
    }
    assert_eq!(server.rows("messages").await, 0);
}

#[tokio::test]
async fn only_members_can_send_and_bodies_are_bounded() {
    let Trio { server, a, channel, .. } = trio(3600).await;
    let outsider = server.phone().await;
    let path = format!("/v1/channels/{channel}/messages");

    assert_eq!(server.send(&outsider, &channel, "hello").await.0, StatusCode::NOT_FOUND);
    assert_eq!(server.anon(POST, &path, Some(json!({ "body": encoded("hello") }))).await.0, StatusCode::UNAUTHORIZED);
    for body in [json!(""), json!("not base64!")] {
        assert_eq!(server.as_phone(&a, POST, &path, Some(json!({ "body": body }))).await.0, StatusCode::BAD_REQUEST);
    }
    let largest = b64(&vec![7u8; deaddropd::MAX_BODY_BYTES]);
    assert_eq!(server.as_phone(&a, POST, &path, Some(json!({ "body": largest }))).await.0, StatusCode::CREATED);
    let too_large = b64(&vec![7u8; deaddropd::MAX_BODY_BYTES + 1]);
    let (status, _) = server.as_phone(&a, POST, &path, Some(json!({ "body": too_large }))).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn a_message_with_nobody_to_receive_it_is_never_stored() {
    let server = Server::new().await;
    let owner = server.phone().await;
    let channel = server.channel(&owner, 3600).await;
    let (status, sent) = server.send(&owner, &channel, "to myself").await;
    assert_eq!((status, &sent["recipients"]), (StatusCode::CREATED, &json!(0)));
    assert_eq!(server.rows("messages").await, 0);
}

#[tokio::test]
async fn deleting_a_channel_wipes_its_queue() {
    let Trio { server, a, b, channel, .. } = trio(3600).await;
    server.send(&a, &channel, "hello").await;
    assert_eq!(server.as_phone(&a, DELETE, &format!("/v1/channels/{channel}"), None).await.0, StatusCode::NO_CONTENT);
    assert_eq!((server.rows("messages").await, server.rows("pending").await), (0, 0));
    assert!(server.queue(&b).await.is_empty());
}

#[tokio::test]
async fn messages_come_out_in_the_order_they_went_in() {
    let Trio { server, a, b, c, channel } = trio(3600).await;
    for text in ["one", "two", "three"] {
        server.send(&a, &channel, text).await;
    }
    server.send(&c, &channel, "four").await;
    let expected: Vec<String> = ["one", "two", "three", "four"].iter().map(|text| encoded(text)).collect();
    assert_eq!(server.collect(&b).await, expected);
}

// ---------------------------------------------------------------- live delivery (WebSocket)

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Runs the server on a real local port; returns the address to connect to.
async fn listen(server: &Server) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = server.router.clone();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("ws://{address}/v1/events")
}

/// The next JSON frame, skipping keepalives. Panics if nothing comes within 5 seconds.
async fn next_frame(socket: &mut Socket) -> Value {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), socket.next()).await.expect("a frame in time");
        if let Message::Text(text) = frame.expect("the connection to stay open").unwrap() {
            return serde_json::from_str(text.as_str()).unwrap();
        }
    }
}

async fn connect(url: &str, token: &str) -> Socket {
    let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    socket.send(Message::text(json!({ "token": token }).to_string())).await.unwrap();
    socket
}

/// Waits (briefly) for the server to have finished handling what was just sent.
async fn eventually(server: &Server, table: &'static str, rows: i64) {
    for _ in 0..100 {
        if server.rows(table).await == rows {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{table} never reached {rows} rows");
}

#[tokio::test]
async fn connected_devices_get_messages_pushed_and_acknowledge_over_the_same_connection() {
    let Trio { server, a, b, c, channel } = trio(3600).await;
    let url = listen(&server).await;
    server.send(&a, &channel, "sent while offline").await;

    let mut socket = connect(&url, &b.token).await;
    assert_eq!(next_frame(&mut socket).await, json!({ "type": "ready" }));
    let first = next_frame(&mut socket).await;
    assert_eq!((&first["type"], &first["body"]), (&json!("message"), &json!(encoded("sent while offline"))));
    assert_eq!((&first["channel"], &first["sender"]), (&json!(channel), &json!(a.id)));

    server.send(&a, &channel, "sent while connected").await;
    let second = next_frame(&mut socket).await;
    assert_eq!(second["body"], json!(encoded("sent while connected")));

    socket.send(Message::text(json!({ "ack": [first["id"], second["id"]] }).to_string())).await.unwrap();
    eventually(&server, "pending", 2).await;
    assert_eq!(server.rows("messages").await, 2, "c has not collected yet");
    server.collect(&c).await;
    assert_eq!(server.rows("messages").await, 0);
}

#[tokio::test]
async fn a_connection_must_sign_in_first() {
    let server = Server::new().await;
    let url = listen(&server).await;
    let mut socket = connect(&url, "made-up").await;
    assert_eq!(next_frame(&mut socket).await["type"], json!("error"));
    assert!(matches!(socket.next().await, None | Some(Ok(Message::Close(_))) | Some(Err(_))));
}

#[tokio::test]
async fn an_owner_is_nudged_when_someone_asks_to_join() {
    let server = Server::new().await;
    let (owner, joiner) = (server.phone().await, server.phone().await);
    let channel = server.channel(&owner, 3600).await;
    let url = listen(&server).await;
    let mut socket = connect(&url, &owner.token).await;
    assert_eq!(next_frame(&mut socket).await, json!({ "type": "ready" }));

    server.ask_to_join(&owner, &channel, &joiner, "Joiner").await;
    assert_eq!(next_frame(&mut socket).await, json!({ "type": "changed" }));
}
