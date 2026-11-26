//! Getting onto a server, signing in, and what the operator can do.

mod common;

use axum::http::StatusCode;
use common::*;
use deaddrop_core::encoding::b64;
use deaddrop_core::keys;
use deaddrop_core::soft::SoftDevice;
use serde_json::json;

#[tokio::test]
async fn a_new_server_lets_in_only_invited_devices() {
    let server = Server::new().await;
    let (_, info) = server.anon(GET, "/v1/server", None).await;
    assert_eq!(info["ways_in"], json!({ "open": false, "token": true, "password": false, "approval": false }));
    assert_eq!(info["server_id"], json!(&*server.state.server_id));

    let (status, _) = server.register(&SoftDevice::new(), json!({})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = server.register(&SoftDevice::new(), json!({ "token": "made-up" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = server.register(&SoftDevice::new(), json!({ "password": "guess" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(server.rows("devices").await, 1, "only the operator is registered");
}

#[tokio::test]
async fn the_operator_token_works_exactly_once() {
    let (state, token) = deaddropd::AppState::open(deaddropd::Store::in_memory().unwrap()).await.unwrap();
    let router = deaddropd::router(state.clone());
    let token = token.unwrap();

    let mut first = registration(&SoftDevice::new());
    first["token"] = json!(token);
    let (status, reply) = call(&router, POST, "/v1/devices", None, Some(first)).await;
    assert_eq!((status, &reply["operator"]), (StatusCode::CREATED, &json!(true)));

    let mut second = registration(&SoftDevice::new());
    second["token"] = json!(token);
    let (status, _) = call(&router, POST, "/v1/devices", None, Some(second)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // And a server that already has an operator does not issue another token on restart.
    let (_, again) = deaddropd::AppState::open(state.store.clone()).await.unwrap();
    assert!(again.is_none());
}

#[tokio::test]
async fn an_invite_is_spent_by_use_and_by_time() {
    let server = Server::new().await;
    let single = server.invite(1, 3600).await;
    let (status, reply) = server.register(&SoftDevice::new(), json!({ "token": single })).await;
    assert_eq!((status, &reply["status"], &reply["operator"]), (StatusCode::CREATED, &json!("active"), &json!(false)));
    let (status, _) = server.register(&SoftDevice::new(), json!({ "token": single })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a single-use invite cannot be used twice");
    assert_eq!(server.rows("tokens").await, 0, "a spent invite is deleted");

    let double = server.invite(2, 3600).await;
    for _ in 0..2 {
        let (status, _) = server.register(&SoftDevice::new(), json!({ "token": double })).await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let (status, _) = server.register(&SoftDevice::new(), json!({ "token": double })).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let stale = server.invite(1, 60).await;
    server.state.clock.advance(61);
    let (status, _) = server.register(&SoftDevice::new(), json!({ "token": stale })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "an expired invite is refused");
}

#[tokio::test]
async fn invites_can_be_switched_off() {
    let server = Server::new().await;
    let invite = server.invite(1, 3600).await;
    let off = json!({ "open": false, "tokens": false, "approval": false });
    let (status, _) = server.as_phone(&server.operator, PUT, "/v1/operator/settings", Some(off)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = server.register(&SoftDevice::new(), json!({ "token": invite })).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_server_password_lets_devices_in_and_guessing_is_slowed() {
    let server = Server::new().await;
    let body = json!({ "password": "correct horse" });
    let (status, _) = server.as_phone(&server.operator, PUT, "/v1/operator/password", Some(body)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, info) = server.anon(GET, "/v1/server", None).await;
    assert_eq!(info["ways_in"]["password"], json!(true));

    let (status, _) = server.register(&SoftDevice::new(), json!({ "password": "correct horse" })).await;
    assert_eq!(status, StatusCode::CREATED);

    for _ in 0..10 {
        let (status, _) = server.register(&SoftDevice::new(), json!({ "password": "wrong" })).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    let (status, _) = server.register(&SoftDevice::new(), json!({ "password": "correct horse" })).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "after ten wrong guesses everyone waits");
    server.state.clock.advance(61);
    let (status, _) = server.register(&SoftDevice::new(), json!({ "password": "correct horse" })).await;
    assert_eq!(status, StatusCode::CREATED);

    // Removing the password closes that way in again.
    let (status, _) =
        server.as_phone(&server.operator, PUT, "/v1/operator/password", Some(json!({ "password": null }))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = server.register(&SoftDevice::new(), json!({ "password": "correct horse" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn an_open_server_lets_anyone_in() {
    let server = Server::new().await;
    let open = json!({ "open": true, "tokens": true, "approval": false });
    server.as_phone(&server.operator, PUT, "/v1/operator/settings", Some(open)).await;
    let (status, reply) = server.register(&SoftDevice::new(), json!({})).await;
    assert_eq!((status, &reply["status"]), (StatusCode::CREATED, &json!("active")));
}

#[tokio::test]
async fn with_approval_on_a_device_waits_for_the_operator() {
    let server = Server::new().await;
    let settings = json!({ "open": false, "tokens": true, "approval": true });
    server.as_phone(&server.operator, PUT, "/v1/operator/settings", Some(settings)).await;

    let device = SoftDevice::new();
    let invite = server.invite(1, 3600).await;
    let (status, reply) = server.register(&device, json!({ "token": invite })).await;
    assert_eq!((status, &reply["status"]), (StatusCode::CREATED, &json!("pending")));

    // It can prove who it is, but nothing else works yet.
    let token = server.sign_in(&device).await.unwrap();
    let (status, reply) = call(&server.router, GET, "/v1/devices/me", Some(&token), None).await;
    assert_eq!((status, &reply["error"]), (StatusCode::FORBIDDEN, &json!("awaiting approval")));

    let (_, list) = server.as_phone(&server.operator, GET, "/v1/operator/devices", None).await;
    let pending: Vec<_> = list["devices"].as_array().unwrap().iter().filter(|d| d["status"] == "pending").collect();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0]["device_id"], json!(device.device_id()));

    let path = format!("/v1/operator/devices/{}/approve", device.device_id());
    let (status, _) = server.as_phone(&server.operator, POST, &path, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(&server.router, GET, "/v1/devices/me", Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn registration_checks_the_keys() {
    let server = Server::new().await;
    let (device, other) = (SoftDevice::new(), SoftDevice::new());

    // An agreement key vouched for by a different signing key.
    let mut body = registration(&device);
    body["agreement_sig"] = json!(other.agreement_signature());
    body["token"] = json!(server.invite(1, 3600).await);
    let (status, _) = server.anon(POST, "/v1/devices", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let mut body = registration(&device);
    body["signing_key"] = json!(b64(b"not a key"));
    let (status, _) = server.anon(POST, "/v1/devices", Some(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn registering_twice_changes_nothing() {
    let server = Server::new().await;
    let phone = server.phone().await;
    let (status, reply) = server.register(&phone.device, json!({})).await;
    assert_eq!((status, &reply["status"]), (StatusCode::OK, &json!("active")));
    assert_eq!(server.rows("devices").await, 2);
}

#[tokio::test]
async fn signing_in_needs_the_devices_own_fresh_signature() {
    let server = Server::new().await;
    let phone = server.phone().await;
    let id = phone.id.clone();
    let challenge = || async {
        let (_, reply) = server.anon(POST, "/v1/sessions/challenge", Some(json!({ "device_id": id }))).await;
        reply["challenge"].as_str().unwrap().to_owned()
    };
    let attempt = |challenge: String, signature: Vec<u8>| {
        let body = json!({ "device_id": id, "challenge": challenge, "signature": b64(&signature) });
        async { server.anon(POST, "/v1/sessions", Some(body)).await.0 }
    };
    let sign = |device: &SoftDevice, server_id: &str, challenge: &str| {
        device.sign(&keys::session_message(server_id, challenge))
    };
    let server_id = server.state.server_id.to_string();

    // Somebody else's key.
    let c = challenge().await;
    assert_eq!(attempt(c.clone(), sign(&SoftDevice::new(), &server_id, &c)).await, StatusCode::UNAUTHORIZED);
    // That failed attempt spent the challenge, even for the right key.
    assert_eq!(attempt(c.clone(), sign(&phone.device, &server_id, &c)).await, StatusCode::UNAUTHORIZED);

    // A signature made for another server.
    let c = challenge().await;
    assert_eq!(attempt(c.clone(), sign(&phone.device, "some-other-server", &c)).await, StatusCode::UNAUTHORIZED);

    // A challenge left too long.
    let c = challenge().await;
    server.state.clock.advance(121);
    assert_eq!(attempt(c.clone(), sign(&phone.device, &server_id, &c)).await, StatusCode::UNAUTHORIZED);

    // The real thing works once, and cannot be replayed.
    let c = challenge().await;
    let signature = sign(&phone.device, &server_id, &c);
    assert_eq!(attempt(c.clone(), signature.clone()).await, StatusCode::OK);
    assert_eq!(attempt(c, signature).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn unregistered_devices_get_a_challenge_but_never_a_session() {
    let server = Server::new().await;
    assert_eq!(server.sign_in(&SoftDevice::new()).await, Err(StatusCode::UNAUTHORIZED));
    assert_eq!(server.rows("challenges").await, 0, "nothing is stored for unknown devices");
}

#[tokio::test]
async fn sessions_expire_and_requests_need_one() {
    let server = Server::new().await;
    let phone = server.phone().await;
    let (status, reply) = server.as_phone(&phone, GET, "/v1/devices/me", None).await;
    assert_eq!((status, &reply["device_id"]), (StatusCode::OK, &json!(phone.id)));

    assert_eq!(server.anon(GET, "/v1/devices/me", None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(call(&server.router, GET, "/v1/devices/me", Some("made-up"), None).await.0, StatusCode::UNAUTHORIZED);

    server.state.clock.advance(3601);
    assert_eq!(server.as_phone(&phone, GET, "/v1/devices/me", None).await.0, StatusCode::UNAUTHORIZED);
    deaddropd::sweep(&server.state).await.unwrap();
    assert_eq!(server.rows("sessions").await, 0, "expired sessions are deleted");
}

#[tokio::test]
async fn only_the_operator_can_use_operator_routes() {
    let server = Server::new().await;
    let phone = server.phone().await;
    let settings = json!({ "open": true, "tokens": true, "approval": false });
    for (method, path, body) in [
        (GET, "/v1/operator/settings".to_owned(), None),
        (PUT, "/v1/operator/settings".to_owned(), Some(settings)),
        (PUT, "/v1/operator/password".to_owned(), Some(json!({ "password": "x" }))),
        (POST, "/v1/operator/invites".to_owned(), Some(json!({}))),
        (GET, "/v1/operator/devices".to_owned(), None),
        (POST, format!("/v1/operator/devices/{}/approve", phone.id), None),
        (DELETE, format!("/v1/operator/devices/{}", server.operator.id), None),
    ] {
        let (status, _) = server.as_phone(&phone, method.clone(), &path, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
    }
}

#[tokio::test]
async fn a_removed_device_is_gone_at_once() {
    let server = Server::new().await;
    let phone = server.phone().await;
    let path = format!("/v1/operator/devices/{}", phone.id);
    assert_eq!(server.as_phone(&server.operator, DELETE, &path, None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(server.as_phone(&phone, GET, "/v1/devices/me", None).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(server.sign_in(&phone.device).await, Err(StatusCode::UNAUTHORIZED));
    assert_eq!(server.rows("sessions").await, 1, "only the operator's session remains");

    // The operator removes themselves the ordinary way, not through the operator route.
    let path = format!("/v1/operator/devices/{}", server.operator.id);
    assert_eq!(server.as_phone(&server.operator, DELETE, &path, None).await.0, StatusCode::CONFLICT);
}

#[tokio::test]
async fn deleting_my_device_deletes_what_it_owned() {
    let server = Server::new().await;
    let (owner, member) = (server.phone().await, server.phone().await);
    let channel = server.channel_with(&owner, 3600, &[(&member, "Member")]).await;
    server.send(&member, &channel, "to the owner").await;
    assert_eq!(server.rows("messages").await, 1);

    assert_eq!(server.as_phone(&owner, DELETE, "/v1/devices/me", None).await.0, StatusCode::NO_CONTENT);
    for table in ["channels", "members", "messages", "pending"] {
        assert_eq!(server.rows(table).await, 0, "{table}");
    }
    let (_, mine) = server.as_phone(&member, GET, "/v1/channels", None).await;
    assert_eq!(mine["channels"], json!([]));
}
