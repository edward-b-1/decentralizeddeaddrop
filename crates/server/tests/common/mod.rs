//! A server in memory and soft "phones" to drive it, shared by the integration tests.
#![allow(dead_code)]

use std::net::{IpAddr, SocketAddr};

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode, header};
use deaddrop_core::encoding::{b64, random_id};
use deaddrop_core::keys;
use deaddrop_core::records::{ChannelRecord, Roster, RosterMember};
use deaddrop_core::soft::SoftDevice;
use deaddropd::{AppState, Store};
use serde_json::{Value, json};
use tower::ServiceExt;

pub const GET: Method = Method::GET;
pub const POST: Method = Method::POST;
pub const PUT: Method = Method::PUT;
pub const DELETE: Method = Method::DELETE;

pub struct Server {
    pub state: AppState,
    pub router: Router,
    /// The device that registered with the operator token.
    pub operator: Phone,
}

/// A registered, signed-in device.
pub struct Phone {
    pub device: SoftDevice,
    pub id: String,
    pub token: String,
}

pub fn registration(device: &SoftDevice) -> Value {
    json!({
        "signing_key": b64(&device.signing_key_der()),
        "agreement_key": b64(&device.agreement_key_der()),
        "agreement_sig": device.agreement_signature(),
    })
}

/// Sends one request straight into a router; returns the status and the JSON body
/// (`Null` when there is none).
pub async fn call(
    router: &Router,
    method: Method,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    call_from(router, None, method, path, token, body).await
}

/// Like [`call`], but as if the request arrived from network address `from`.
pub async fn call_from(
    router: &Router,
    from: Option<IpAddr>,
    method: Method,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let request = match body {
        Some(body) => request.header(header::CONTENT_TYPE, "application/json").body(Body::from(body.to_string())),
        None => request.body(Body::empty()),
    }
    .unwrap();
    let mut request = request;
    if let Some(address) = from {
        request.extensions_mut().insert(ConnectInfo(SocketAddr::new(address, 40000)));
    }
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

impl Server {
    pub async fn new() -> Server {
        let (state, operator_token) = AppState::open(Store::in_memory().unwrap()).await.unwrap();
        let router = deaddropd::router(state.clone());
        let device = SoftDevice::new();
        let mut body = registration(&device);
        body["token"] = json!(operator_token.expect("a new server issues an operator token"));
        let (status, reply) = call(&router, POST, "/v1/devices", None, Some(body)).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(reply["operator"], json!(true));
        let token = sign_in(&router, &state, &device).await.unwrap();
        let operator = Phone { id: device.device_id(), device, token };
        Server { state, router, operator }
    }

    /// A request from nobody in particular.
    pub async fn anon(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        call(&self.router, method, path, None, body).await
    }

    /// A request from a signed-in phone.
    pub async fn as_phone(
        &self,
        phone: &Phone,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        call(&self.router, method, path, Some(&phone.token), body).await
    }

    /// A server invite token from the operator.
    pub async fn invite(&self, uses: i64, ttl: i64) -> String {
        let body = json!({ "uses": uses, "ttl": ttl });
        let (status, reply) = self.as_phone(&self.operator, POST, "/v1/operator/invites", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED);
        reply["token"].as_str().unwrap().to_owned()
    }

    pub async fn register(&self, device: &SoftDevice, extra: Value) -> (StatusCode, Value) {
        self.register_from(None, device, extra).await
    }

    /// Registers as if from network address `from`.
    pub async fn register_from(&self, from: Option<&str>, device: &SoftDevice, extra: Value) -> (StatusCode, Value) {
        let mut body = registration(device);
        for (key, value) in extra.as_object().unwrap() {
            body[key] = value.clone();
        }
        let from = from.map(|address| address.parse().unwrap());
        call_from(&self.router, from, POST, "/v1/devices", None, Some(body)).await
    }

    pub async fn sign_in(&self, device: &SoftDevice) -> Result<String, StatusCode> {
        sign_in(&self.router, &self.state, device).await
    }

    /// A new phone, let in with a fresh invite and signed in.
    pub async fn phone(&self) -> Phone {
        let device = SoftDevice::new();
        let token = self.invite(1, 3600).await;
        let (status, _) = self.register(&device, json!({ "token": token })).await;
        assert_eq!(status, StatusCode::CREATED);
        let token = self.sign_in(&device).await.unwrap();
        Phone { id: device.device_id(), device, token }
    }

    /// `owner` creates a channel with themselves as the only member; returns its ID.
    pub async fn channel(&self, owner: &Phone, max_age: u64) -> String {
        let id = random_id();
        let body = json!({
            "channel": owner.device.seal(&channel_record(owner, &id, max_age, 1)),
            "roster": owner.device.seal(&roster(&id, 1, &[(owner, "Owner")])),
        });
        let (status, reply) = self.as_phone(owner, POST, "/v1/channels", Some(body)).await;
        assert_eq!(status, StatusCode::CREATED, "{reply}");
        id
    }

    /// `phone` asks to join `channel` with a fresh invite from `owner`. The stamp
    /// stands in for the one a phone makes with the invite's secret: the server keeps
    /// it and hands it on, and has no means of checking it.
    pub async fn ask_to_join(&self, owner: &Phone, channel: &str, phone: &Phone, name: &str) {
        let path = format!("/v1/channels/{channel}/invites");
        let (status, reply) = self.as_phone(owner, POST, &path, Some(json!({}))).await;
        assert_eq!(status, StatusCode::CREATED);
        let body = json!({ "token": reply["token"], "name": name, "stamp": format!("stamp of {name}") });
        let (status, _) =
            self.as_phone(phone, POST, &format!("/v1/channels/{channel}/join-requests"), Some(body)).await;
        assert_eq!(status, StatusCode::ACCEPTED);
    }

    pub async fn put_roster(&self, owner: &Phone, channel: &str, roster: &Roster) -> (StatusCode, Value) {
        let body = json!({ "roster": owner.device.seal(roster) });
        self.as_phone(owner, PUT, &format!("/v1/channels/{channel}/roster"), Some(body)).await
    }

    /// A channel owned by `owner` with `others` already let in (roster version 2).
    pub async fn channel_with(&self, owner: &Phone, max_age: u64, others: &[(&Phone, &str)]) -> String {
        let channel = self.channel(owner, max_age).await;
        let mut members = vec![(owner, "Owner")];
        for &(phone, name) in others {
            self.ask_to_join(owner, &channel, phone, name).await;
            members.push((phone, name));
        }
        let (status, reply) = self.put_roster(owner, &channel, &roster(&channel, 2, &members)).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{reply}");
        channel
    }

    /// Sends `text` (standing in for a sealed body) to everyone else in the channel.
    pub async fn send(&self, from: &Phone, channel: &str, text: &str) -> (StatusCode, Value) {
        let body = json!({ "body": b64(text.as_bytes()) });
        self.as_phone(from, POST, &format!("/v1/channels/{channel}/messages"), Some(body)).await
    }

    /// The IDs and bodies waiting in a phone's queue.
    pub async fn queue(&self, phone: &Phone) -> Vec<(String, String)> {
        let (status, reply) = self.as_phone(phone, GET, "/v1/queue", None).await;
        assert_eq!(status, StatusCode::OK);
        reply["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| (message["id"].as_str().unwrap().to_owned(), message["body"].as_str().unwrap().to_owned()))
            .collect()
    }

    pub async fn ack(&self, phone: &Phone, ids: &[String]) {
        let (status, _) = self.as_phone(phone, POST, "/v1/queue/ack", Some(json!({ "ids": ids }))).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    /// Fetches and acknowledges everything waiting for `phone`; returns the bodies.
    pub async fn collect(&self, phone: &Phone) -> Vec<String> {
        let waiting = self.queue(phone).await;
        let ids: Vec<String> = waiting.iter().map(|(id, _)| id.clone()).collect();
        self.ack(phone, &ids).await;
        waiting.into_iter().map(|(_, body)| body).collect()
    }

    /// How many rows a table holds: what is really on the server's disk.
    pub async fn rows(&self, table: &'static str) -> i64 {
        self.state
            .store
            .run(move |db| Ok(db.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row.get(0))?))
            .await
            .unwrap()
    }
}

pub async fn sign_in(router: &Router, state: &AppState, device: &SoftDevice) -> Result<String, StatusCode> {
    let id = device.device_id();
    let (status, reply) = call(router, POST, "/v1/sessions/challenge", None, Some(json!({ "device_id": id }))).await;
    assert_eq!(status, StatusCode::OK);
    let challenge = reply["challenge"].as_str().unwrap();
    let signature = b64(&device.sign(&keys::session_message(&state.server_id, challenge)));
    let body = json!({ "device_id": id, "challenge": challenge, "signature": signature });
    let (status, reply) = call(router, POST, "/v1/sessions", None, Some(body)).await;
    if status == StatusCode::OK { Ok(reply["token"].as_str().unwrap().to_owned()) } else { Err(status) }
}

pub fn channel_record(owner: &Phone, id: &str, max_age: u64, version: u64) -> ChannelRecord {
    ChannelRecord {
        v: 1,
        id: id.to_owned(),
        name: "Test channel".into(),
        owner: owner.id.clone(),
        max_age,
        version,
        issued: 1_800_000_000,
    }
}

pub fn roster(channel: &str, version: u64, members: &[(&Phone, &str)]) -> Roster {
    Roster {
        v: 1,
        channel: channel.to_owned(),
        version,
        members: members
            .iter()
            .map(|(phone, name)| RosterMember { device: phone.id.clone(), name: (*name).to_owned(), since: 0 })
            .collect(),
        issued: 1_800_000_000,
    }
}

pub fn encoded(text: &str) -> String {
    b64(text.as_bytes())
}

/// A scratch directory that is removed again when the test ends.
pub struct Scratch(pub std::path::PathBuf);

impl Scratch {
    pub fn new() -> Scratch {
        let path = std::env::temp_dir().join(format!("deaddrop-test-{}", random_id()));
        std::fs::create_dir(&path).unwrap();
        Scratch(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
