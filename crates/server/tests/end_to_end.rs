//! Whole clients (hardware-style device keys plus a `Messenger`) talking through a real
//! server: what the app will do, including every check a phone must make for itself.

mod common;

use axum::http::StatusCode;
use common::*;
use deaddrop_core::encoding::{b64, unb64};
use deaddrop_core::keys;
use deaddrop_core::records::{Roster, Signed, SignedKey, open_messaging_keys};
use deaddrop_crypto::{Error, Incoming, Messenger};
use serde_json::{Value, json};

struct Client {
    phone: Phone,
    messenger: Messenger,
}

/// Why a client refused to go on.
#[derive(Debug, PartialEq)]
enum Refused {
    /// The channel's owner is not who the invite said.
    WrongOwner,
    /// The roster is not signed by the owner.
    BadRoster,
    /// A member's keys do not trace back to the roster.
    BadMemberKeys(String),
    BadPrekey(String),
    /// A member has published no prekey at all.
    NoPrekey(String),
}

impl Client {
    /// Registers a device and publishes its messaging keys and some prekeys.
    async fn new(server: &Server) -> Client {
        let phone = server.phone().await;
        let mut messenger = Messenger::new(&phone.id);
        let prekeys = messenger.new_prekeys(5, true);
        let body = json!({
            "keys": phone.device.seal(&messenger.keys(server.state.clock.now())),
            "one_time": prekeys.one_time,
            "fallback": prekeys.fallback,
        });
        let (status, reply) = server.as_phone(&phone, PUT, "/v1/keys", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{reply}");
        assert_eq!(reply, json!({ "published": true, "one_time": 5, "fallback": true }));
        Client { phone, messenger }
    }

    /// Fetches a channel's roster and members' keys, trusting nothing the server says:
    /// - the owner's key must hash to `owner`, the ID that came with the invite;
    /// - the roster must be signed by that key;
    /// - each member's signing key must hash to their ID in the roster, and their
    ///   messaging keys must be signed by it.
    async fn sync(&mut self, server: &Server, channel: &str, owner: &str) -> Result<(), Refused> {
        let (status, shown) = server.as_phone(&self.phone, GET, &format!("/v1/channels/{channel}"), None).await;
        assert_eq!(status, StatusCode::OK);
        let owner_key = unb64(shown["owner_key"].as_str().unwrap()).unwrap();
        if keys::device_id(&owner_key) != owner {
            return Err(Refused::WrongOwner);
        }
        let (status, view) = server.as_phone(&self.phone, GET, &format!("/v1/channels/{channel}/members"), None).await;
        assert_eq!(status, StatusCode::OK);
        let signed: Signed = serde_json::from_value(view["roster"].clone()).unwrap();
        let roster: Roster = signed.open(&owner_key).map_err(|_| Refused::BadRoster)?;
        if roster.channel != channel {
            return Err(Refused::BadRoster);
        }
        let left: Vec<&str> = view["left"].as_array().unwrap().iter().map(|id| id.as_str().unwrap()).collect();

        let mut members = Vec::new();
        for member in &roster.members {
            if left.contains(&member.device.as_str()) {
                continue;
            }
            members.push(member.device.clone());
            if member.device == self.phone.id {
                continue;
            }
            let bad = || Refused::BadMemberKeys(member.device.clone());
            let entry = view["keys"].as_array().unwrap().iter().find(|entry| entry["device_id"] == member.device);
            let entry = entry.ok_or_else(bad)?;
            let signing_key = unb64(entry["signing_key"].as_str().unwrap()).ok_or_else(bad)?;
            let signed: Signed = serde_json::from_value(entry["messaging_keys"].clone()).map_err(|_| bad())?;
            let messaging = open_messaging_keys(&member.device, &signing_key, &signed).map_err(|_| bad())?;
            self.messenger.add_peer(&messaging).map_err(|_| bad())?;
        }
        self.messenger.set_roster(channel, roster.version, &members);
        Ok(())
    }

    /// Claims a prekey for every member there is no private session with yet.
    async fn connect(&mut self, server: &Server, channel: &str) -> Result<(), Refused> {
        let missing = self.messenger.missing_sessions(channel).unwrap();
        if missing.is_empty() {
            return Ok(());
        }
        let path = format!("/v1/channels/{channel}/keys/claim");
        let (status, reply) = server.as_phone(&self.phone, POST, &path, Some(json!({ "devices": missing }))).await;
        assert_eq!(status, StatusCode::OK, "{reply}");
        for claimed in reply["claimed"].as_array().unwrap() {
            let device = claimed["device_id"].as_str().unwrap();
            let prekey: SignedKey =
                serde_json::from_value(claimed["prekey"].clone()).map_err(|_| Refused::NoPrekey(device.to_owned()))?;
            self.messenger.start_session(device, &prekey).map_err(|_| Refused::BadPrekey(device.to_owned()))?;
        }
        Ok(())
    }

    async fn send(&mut self, server: &Server, channel: &str, text: &str) {
        self.connect(server, channel).await.unwrap();
        let outgoing = self.messenger.encrypt(channel, text.as_bytes(), server.state.clock.now()).unwrap();
        let path = format!("/v1/channels/{channel}/messages");
        for (device, body) in &outgoing.shares {
            let body = json!({ "body": b64(body), "to": [device] });
            assert_eq!(server.as_phone(&self.phone, POST, &path, Some(body)).await.0, StatusCode::CREATED);
        }
        let body = json!({ "body": b64(&outgoing.message) });
        assert_eq!(server.as_phone(&self.phone, POST, &path, Some(body)).await.0, StatusCode::CREATED);
    }

    /// Collects the queue: opens everything, acknowledges it, returns the texts read.
    async fn receive(&mut self, server: &Server) -> Vec<String> {
        let (status, reply) = server.as_phone(&self.phone, GET, "/v1/queue", None).await;
        assert_eq!(status, StatusCode::OK);
        let mut texts = Vec::new();
        let mut ids = Vec::new();
        for message in reply["messages"].as_array().unwrap() {
            let channel = message["channel"].as_str().unwrap();
            let sender = message["sender"].as_str().unwrap();
            let body = unb64(message["body"].as_str().unwrap()).unwrap();
            match self.messenger.decrypt(channel, sender, &body) {
                Ok(Incoming::Message(plaintext)) => texts.push(String::from_utf8(plaintext).unwrap()),
                Ok(Incoming::KeysReceived | Incoming::KeysWanted) => {}
                Err(error) => panic!("a queued message did not open: {error:?}"),
            }
            ids.push(message["id"].clone());
        }
        let (status, _) = server.as_phone(&self.phone, POST, "/v1/queue/ack", Some(json!({ "ids": ids }))).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        texts
    }
}

/// Every sealed body the server is holding right now.
async fn stored_bodies(server: &Server) -> Vec<Vec<u8>> {
    server
        .state
        .store
        .run(|db| {
            let mut query = db.prepare("SELECT body FROM messages ORDER BY seq")?;
            let bodies = query.query_map([], |row| row.get(0))?.collect::<rusqlite::Result<Vec<Vec<u8>>>>()?;
            Ok(bodies)
        })
        .await
        .unwrap()
}

/// Three clients in a channel owned by the first, all synced.
async fn trio(server: &Server) -> (Client, Client, Client, String) {
    let (mut a, mut b, mut c) = (Client::new(server).await, Client::new(server).await, Client::new(server).await);
    let channel = server.channel_with(&a.phone, 3600, &[(&b.phone, "Ben"), (&c.phone, "Cara")]).await;
    let owner = a.phone.id.clone();
    for client in [&mut a, &mut b, &mut c] {
        client.sync(server, &channel, &owner).await.unwrap();
    }
    (a, b, c, channel)
}

#[tokio::test]
async fn three_phones_talk_through_a_server_that_only_ever_holds_sealed_bytes() {
    let server = Server::new().await;
    let (mut a, mut b, mut c, channel) = trio(&server).await;
    let secret = "the eagle lands at midnight";

    a.send(&server, &channel, secret).await;
    // While it waits in the queue, the server has it only in sealed form.
    let bodies = stored_bodies(&server).await;
    assert_eq!(bodies.len(), 3, "a hand-over for each of the two others, and the message");
    for body in &bodies {
        assert!(!body.windows(secret.len()).any(|window| window == secret.as_bytes()));
    }

    assert_eq!(b.receive(&server).await, [secret]);
    assert_eq!(c.receive(&server).await, [secret]);
    assert_eq!(server.rows("messages").await, 0, "delivered to all, so wiped");

    b.send(&server, &channel, "ben replies").await;
    c.send(&server, &channel, "so does cara").await;
    a.send(&server, &channel, "anna again").await;
    assert_eq!(a.receive(&server).await, ["ben replies", "so does cara"]);
    assert_eq!(b.receive(&server).await, ["so does cara", "anna again"]);
    assert_eq!(c.receive(&server).await, ["ben replies", "anna again"]);
    assert_eq!(server.rows("messages").await, 0);
}

#[tokio::test]
async fn a_removed_member_is_locked_out_by_the_server_and_by_the_keys() {
    let server = Server::new().await;
    let (mut a, mut b, mut c, channel) = trio(&server).await;
    a.send(&server, &channel, "before").await;
    assert_eq!(b.receive(&server).await, ["before"]);
    assert_eq!(c.receive(&server).await, ["before"]);

    let without_cara = roster(&channel, 3, &[(&a.phone, "Owner"), (&b.phone, "Ben")]);
    assert_eq!(server.put_roster(&a.phone, &channel, &without_cara).await.0, StatusCode::NO_CONTENT);
    let owner = a.phone.id.clone();
    a.sync(&server, &channel, &owner).await.unwrap();
    b.sync(&server, &channel, &owner).await.unwrap();

    a.send(&server, &channel, "after").await;
    // First barrier: the server queues nothing for cara.
    assert_eq!(c.receive(&server).await, Vec::<String>::new());
    // Second barrier: even a server that leaked every sealed body to cara would not
    // help her. Nothing it holds opens with any key she has.
    for body in stored_bodies(&server).await {
        assert!(c.messenger.decrypt(&channel, &a.phone.id, &body).is_err());
    }
    assert_eq!(b.receive(&server).await, ["after"]);
}

#[tokio::test]
async fn a_new_member_joins_and_reads_only_what_comes_after() {
    let server = Server::new().await;
    let (mut a, mut b) = (Client::new(&server).await, Client::new(&server).await);
    let channel = server.channel_with(&a.phone, 3600, &[(&b.phone, "Ben")]).await;
    let owner = a.phone.id.clone();
    a.sync(&server, &channel, &owner).await.unwrap();
    b.sync(&server, &channel, &owner).await.unwrap();
    a.send(&server, &channel, "before dan").await;
    let earlier = stored_bodies(&server).await;
    assert_eq!(b.receive(&server).await, ["before dan"]);

    let mut d = Client::new(&server).await;
    server.ask_to_join(&a.phone, &channel, &d.phone, "Dan").await;
    let with_dan = roster(&channel, 3, &[(&a.phone, "Owner"), (&b.phone, "Ben"), (&d.phone, "Dan")]);
    assert_eq!(server.put_roster(&a.phone, &channel, &with_dan).await.0, StatusCode::NO_CONTENT);
    for client in [&mut a, &mut b, &mut d] {
        client.sync(&server, &channel, &owner).await.unwrap();
    }

    b.send(&server, &channel, "welcome dan").await;
    assert_eq!(d.receive(&server).await, ["welcome dan"]);
    assert_eq!(a.receive(&server).await, ["welcome dan"]);
    d.send(&server, &channel, "thanks").await;
    assert_eq!(a.receive(&server).await, ["thanks"]);
    assert_eq!(b.receive(&server).await, ["thanks"]);
    for body in earlier {
        assert!(d.messenger.decrypt(&channel, &a.phone.id, &body).is_err(), "nothing from before dan joined opens");
    }
}

#[tokio::test]
async fn a_server_that_swaps_keys_is_caught() {
    let server = Server::new().await;
    let (mut a, b, c, channel) = trio(&server).await;
    let owner = a.phone.id.clone();
    let (ben, cara) = (b.phone.id.clone(), c.phone.id.clone());
    let run = |sql: &'static str, from: String, to: String| {
        server.state.store.run(move |db| Ok(db.execute(sql, [from, to])?))
    };

    // Ben's prekeys replaced with cara's (properly signed, but by cara).
    run("DELETE FROM prekeys WHERE device = ?2 AND ?1 = ?1", cara.clone(), ben.clone()).await.unwrap();
    run(
        "INSERT INTO prekeys (device, key, signature, fallback)
         SELECT ?2, key, signature, fallback FROM prekeys WHERE device = ?1",
        cara.clone(),
        ben.clone(),
    )
    .await
    .unwrap();
    assert_eq!(a.connect(&server, &channel).await, Err(Refused::BadPrekey(ben.clone())));

    // Ben's messaging keys replaced with cara's signed record.
    let mut fresh = Client::new(&server).await;
    server.ask_to_join(&a.phone, &channel, &fresh.phone, "Fresh").await;
    let all =
        roster(&channel, 3, &[(&a.phone, "Owner"), (&b.phone, "Ben"), (&c.phone, "Cara"), (&fresh.phone, "Fresh")]);
    assert_eq!(server.put_roster(&a.phone, &channel, &all).await.0, StatusCode::NO_CONTENT);
    run(
        "UPDATE messaging_keys SET (record, signature) =
            (SELECT record, signature FROM messaging_keys WHERE device = ?1) WHERE device = ?2",
        cara.clone(),
        ben.clone(),
    )
    .await
    .unwrap();
    assert_eq!(fresh.sync(&server, &channel, &owner).await, Err(Refused::BadMemberKeys(ben.clone())));

    // Ben's signing key replaced as well, so the record's signature would check out:
    // the key no longer hashes to the ID the owner signed.
    run(
        "UPDATE devices SET signing_key = (SELECT signing_key FROM devices WHERE id = ?1) WHERE id = ?2",
        cara.clone(),
        ben.clone(),
    )
    .await
    .unwrap();
    assert_eq!(fresh.sync(&server, &channel, &owner).await, Err(Refused::BadMemberKeys(ben.clone())));

    // And a channel offered under the wrong owner is refused outright.
    assert_eq!(fresh.sync(&server, &channel, &cara).await, Err(Refused::WrongOwner));
}

#[tokio::test]
async fn a_one_time_prekey_is_handed_out_once_and_then_the_fallback() {
    let server = Server::new().await;
    let owner = Client::new(&server).await;
    let target = Client::new(&server).await;
    let mut members = vec![(&owner.phone, "Owner"), (&target.phone, "Target")];
    let others: Vec<Phone> = futures_util::future::join_all((0..6).map(|_| server.phone())).await;
    let names: Vec<String> = (0..6).map(|n| format!("Member {n}")).collect();
    let channel = server.channel(&owner.phone, 3600).await;
    server.ask_to_join(&owner.phone, &channel, &target.phone, "Target").await;
    for (phone, name) in others.iter().zip(&names) {
        server.ask_to_join(&owner.phone, &channel, phone, name).await;
        members.push((phone, name));
    }
    assert_eq!(
        server.put_roster(&owner.phone, &channel, &roster(&channel, 2, &members)).await.0,
        StatusCode::NO_CONTENT
    );

    let path = format!("/v1/channels/{channel}/keys/claim");
    let mut seen = Vec::new();
    for (n, phone) in others.iter().enumerate() {
        let (status, reply) = server.as_phone(phone, POST, &path, Some(json!({ "devices": [target.phone.id] }))).await;
        assert_eq!(status, StatusCode::OK);
        let claimed = &reply["claimed"][0];
        assert_eq!(claimed["fallback"], json!(n >= 5), "five one-time prekeys, then the fallback");
        if n < 5 {
            assert!(!seen.contains(&claimed["prekey"]["key"]), "no one-time prekey is handed out twice");
            seen.push(claimed["prekey"]["key"].clone());
        }
    }
    let (_, left) = server.as_phone(&target.phone, GET, "/v1/keys", None).await;
    assert_eq!(left, json!({ "published": true, "one_time": 0, "fallback": true }), "time to upload more");
}

#[tokio::test]
async fn prekeys_are_only_for_fellow_members_and_keys_only_from_their_owner() {
    let server = Server::new().await;
    let (a, b, _c, channel) = trio(&server).await;
    let outsider = Client::new(&server).await;
    let path = format!("/v1/channels/{channel}/keys/claim");

    let claim = json!({ "devices": [b.phone.id] });
    assert_eq!(server.as_phone(&outsider.phone, POST, &path, Some(claim)).await.0, StatusCode::NOT_FOUND);
    for devices in [json!([outsider.phone.id]), json!([a.phone.id]), json!(["nobody"])] {
        let (status, _) = server.as_phone(&a.phone, POST, &path, Some(json!({ "devices": devices }))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    let (_, left) = server.as_phone(&b.phone, GET, "/v1/keys", None).await;
    assert_eq!(left["one_time"], json!(5), "none of that used up a prekey");

    // Publishing someone else's keys, or keys signed by someone else, is refused.
    let others: Value = json!({ "keys": b.phone.device.seal(&b.messenger.keys(0)) });
    assert_eq!(server.as_phone(&a.phone, PUT, "/v1/keys", Some(others)).await.0, StatusCode::BAD_REQUEST);
    let forged: Value = json!({ "keys": b.phone.device.seal(&a.messenger.keys(0)) });
    assert_eq!(server.as_phone(&a.phone, PUT, "/v1/keys", Some(forged)).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn publishing_new_messaging_keys_discards_prekeys_made_with_the_old_ones() {
    let server = Server::new().await;
    let client = Client::new(&server).await;
    let mut replacement = Messenger::new(&client.phone.id);
    let prekeys = replacement.new_prekeys(2, false);
    let body = json!({ "keys": client.phone.device.seal(&replacement.keys(0)), "one_time": prekeys.one_time });
    let (status, reply) = server.as_phone(&client.phone, PUT, "/v1/keys", Some(body)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reply, json!({ "published": true, "one_time": 2, "fallback": false }));
}

#[tokio::test]
async fn a_client_restored_from_storage_carries_on() {
    let server = Server::new().await;
    let (mut a, mut b, mut c, channel) = trio(&server).await;
    a.send(&server, &channel, "one").await;
    assert_eq!(b.receive(&server).await, ["one"]);

    // Ben's app is closed and reopened; cara stays offline throughout.
    b.messenger = Messenger::from_bytes(&b.messenger.to_bytes()).unwrap();
    a.send(&server, &channel, "two").await;
    assert_eq!(b.receive(&server).await, ["two"]);
    b.send(&server, &channel, "three").await;
    assert_eq!(a.receive(&server).await, ["three"]);
    assert_eq!(c.receive(&server).await, ["one", "two", "three"]);
    assert_eq!(c.messenger.decrypt(&channel, &a.phone.id, b"\x02junk"), Err(Error::Malformed));
}
