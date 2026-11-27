//! One identity on one server.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

use deaddrop_core::encoding::{b64, random_id, unb64};
use deaddrop_core::keys;
use deaddrop_core::records::{ChannelRecord, Roster, RosterMember, Signed, SignedKey, open_messaging_keys};
use deaddrop_crypto::{Error as CryptoError, Incoming, Messenger};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::http::Api;
use crate::link::{ChannelInvite, Link, ServerAddress};
use crate::live::Live;
use crate::{ClientError, DeviceKeys, key_b64};

/// Prekeys the server should hold for this device, and when to top them up.
const PREKEY_TARGET: usize = 50;
const PREKEY_LOW: u64 = 25;

/// Called with the account's whole state whenever it must be stored before the engine
/// goes on. The bytes are secret key material: encrypt them before they touch a disk.
pub type Save<'a> = &'a mut dyn FnMut(&[u8]);

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// How a new device gets onto a server.
#[derive(Default, Clone, Debug)]
pub struct WayIn {
    pub token: Option<String>,
    pub password: Option<String>,
}

/// Which ways onto a server are switched on (docs/PROTOCOL.md section 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaysIn {
    pub open: bool,
    pub tokens: bool,
    pub approval: bool,
    /// Whether a server password is set.
    pub password: bool,
}

/// A channel as this device knows it. Everything here has been checked against the
/// owner's signature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    pub id: String,
    pub name: String,
    /// Device ID of the owner.
    pub owner: String,
    /// Maximum message age, in seconds.
    pub max_age: u64,
    pub record_version: u64,
    pub roster_version: u64,
    /// Everyone in the owner's signed roster.
    pub members: Vec<RosterMember>,
    /// Members who have left since the roster was signed.
    pub left: Vec<String>,
}

impl Channel {
    pub fn name_of(&self, device: &str) -> Option<&str> {
        self.members.iter().find(|member| member.device == device).map(|member| member.name.as_str())
    }
}

/// A device registered on a server, as its operator sees it: an ID and nothing else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerDevice {
    pub device: String,
    /// Registered but not yet approved.
    pub waiting: bool,
    pub operator: bool,
}

/// How long to wait before asking the same member for the same channel's keys again.
const ASK_AGAIN_SECONDS: i64 = 60;

/// Someone asking to join a channel this account owns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinRequest {
    pub device: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Message {
        channel: String,
        /// Device ID of the sender, as proved by the encryption (not the server's word).
        sender: String,
        /// The sender's name in this channel, from the owner's roster.
        sender_name: Option<String>,
        text: String,
        /// When the sender says they sent it, Unix seconds.
        sent: i64,
    },
    /// A message arrived that cannot be opened, for example because its keys expired
    /// from the queue before this device collected them.
    Unreadable { channel: String, sender: String },
    /// The list of channels, or something about one of them, has changed.
    ChannelsChanged,
}

/// A channel this device has asked to join. The owner's ID comes from the invite, and
/// is what the channel is checked against once the owner lets the device in.
#[derive(Clone, Serialize, Deserialize)]
struct Asked {
    channel: String,
    owner: String,
}

/// Something sealed and waiting to be posted. Kept in the saved state so that nothing
/// encrypted is ever lost between being made and being sent.
#[derive(Clone, Serialize, Deserialize)]
struct Outgoing {
    channel: String,
    /// One member (a key hand-over) or everyone (a message).
    to: Option<String>,
    body: String,
}

struct Session {
    token: String,
    expires: i64,
}

pub struct Account {
    address: ServerAddress,
    api: Api,
    device: String,
    server_id: String,
    operator: bool,
    keys_published: bool,
    session: Option<Session>,
    messenger: Messenger,
    channels: BTreeMap<String, Channel>,
    asked: Vec<Asked>,
    outbox: VecDeque<Outgoing>,
    /// When each (channel, member) was last asked to hand their keys over again.
    asked_keys: HashMap<(String, String), i64>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    v: u32,
    address: ServerAddress,
    device: String,
    server_id: String,
    operator: bool,
    keys_published: bool,
    messenger: String,
    channels: Vec<Channel>,
    asked: Vec<Asked>,
    outbox: Vec<Outgoing>,
}

/// The server's rule for names: some characters, no spaces at the ends, nothing unprintable.
fn valid_name(name: &str, max_chars: usize) -> bool {
    let count = name.chars().count();
    count >= 1 && count <= max_chars && name.trim() == name && !name.chars().any(char::is_control)
}

/// Whether `device` could be called `name` in a channel: names are unique in a channel,
/// whatever the capitals.
fn name_is_free(channel: &Channel, device: &str, name: &str) -> Result<(), ClientError> {
    if !valid_name(name, 32) {
        return Err(ClientError::Invalid("a name is 1 to 32 characters"));
    }
    let taken = channel
        .members
        .iter()
        .any(|member| member.device != device && member.name.to_lowercase() == name.to_lowercase());
    if taken { Err(ClientError::Invalid("someone else in this channel has that name")) } else { Ok(()) }
}

fn text(value: &Value, what: &'static str) -> Result<String, ClientError> {
    value.as_str().map(str::to_owned).ok_or(ClientError::Untrusted(what))
}

fn signed(value: &Value, what: &'static str) -> Result<Signed, ClientError> {
    serde_json::from_value(value.clone()).map_err(|_| ClientError::Untrusted(what))
}

impl Account {
    // ------------------------------------------------------------------ getting in

    /// Registers this device on a server and, if it is let straight in, publishes its
    /// messaging keys. Where the operator must approve new devices the account comes
    /// back not yet [`Account::approved`]; [`Account::sync`] finishes the job later.
    pub fn join(address: &ServerAddress, way_in: &WayIn, keys: &dyn DeviceKeys) -> Result<Account, ClientError> {
        let api = Api::new(address)?;
        let info = api.call(Method::GET, "/v1/server", None, None)?;
        let server_id = text(&info["server_id"], "server description")?;

        let mut body = json!({
            "signing_key": key_b64(&keys.signing_key_der()),
            "agreement_key": key_b64(&keys.agreement_key_der()),
            "agreement_sig": keys.agreement_signature(),
        });
        if let Some(token) = &way_in.token {
            body["token"] = json!(token);
        } else if let Some(password) = &way_in.password {
            body["password"] = json!(password);
        }
        let reply = api.call(Method::POST, "/v1/devices", None, Some(body))?;

        let device = keys::device_id(&keys.signing_key_der());
        let mut account = Account {
            address: address.clone(),
            api,
            messenger: Messenger::new(&device),
            device,
            server_id,
            operator: reply["operator"] == true,
            keys_published: false,
            session: None,
            channels: BTreeMap::new(),
            asked: Vec::new(),
            outbox: VecDeque::new(),
            asked_keys: HashMap::new(),
        };
        if reply["status"] == "active" {
            account.publish_keys(keys)?;
        }
        Ok(account)
    }

    pub fn address(&self) -> &ServerAddress {
        &self.address
    }

    pub fn device(&self) -> &str {
        &self.device
    }

    /// Whether this device runs the server.
    pub fn operator(&self) -> bool {
        self.operator
    }

    /// False while the device is still waiting for the operator's approval.
    pub fn approved(&self) -> bool {
        self.keys_published
    }

    pub fn channels(&self) -> impl Iterator<Item = &Channel> {
        self.channels.values()
    }

    pub fn channel(&self, id: &str) -> Option<&Channel> {
        self.channels.get(id)
    }

    /// Channels this device has asked to join and not yet been let into.
    pub fn asked(&self) -> impl Iterator<Item = &str> {
        self.asked.iter().map(|asked| asked.channel.as_str())
    }

    /// Sealed messages made but not yet posted (the server could not be reached).
    pub fn unsent(&self) -> usize {
        self.outbox.len()
    }

    // ------------------------------------------------------------------ signing in

    fn token(&mut self, keys: &dyn DeviceKeys) -> Result<String, ClientError> {
        if let Some(session) = &self.session
            && session.expires > now()
        {
            return Ok(session.token.clone());
        }
        let asked = json!({ "device_id": self.device });
        let reply = self.api.call(Method::POST, "/v1/sessions/challenge", None, Some(asked))?;
        let challenge = text(&reply["challenge"], "sign-in challenge")?;
        let signature = b64(&keys.sign(&keys::session_message(&self.server_id, &challenge)));
        let body = json!({ "device_id": self.device, "challenge": challenge, "signature": signature });
        let reply = self.api.call(Method::POST, "/v1/sessions", None, Some(body))?;
        let token = text(&reply["token"], "session")?;
        // Renew a minute early rather than be caught out mid-request.
        let expires = now() + reply["expires_in"].as_i64().unwrap_or(0) - 60;
        self.session = Some(Session { token: token.clone(), expires });
        Ok(token)
    }

    /// A signed-in request. A session the server no longer accepts is renewed once.
    fn request(
        &mut self,
        keys: &dyn DeviceKeys,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, ClientError> {
        for attempt in 0..2 {
            let token = self.token(keys)?;
            match self.api.call(method.clone(), path, Some(&token), body.clone()) {
                Err(ClientError::Refused { status: 401, .. }) if attempt == 0 => self.session = None,
                Err(ClientError::Refused { status: 403, message }) if message == "awaiting approval" => {
                    return Err(ClientError::AwaitingApproval);
                }
                other => return other,
            }
        }
        unreachable!("the second attempt always returns")
    }

    // ------------------------------------------------------------------ own keys

    fn publish_keys(&mut self, keys: &dyn DeviceKeys) -> Result<(), ClientError> {
        let prekeys = self.messenger.new_prekeys(PREKEY_TARGET, true);
        self.upload_keys(keys, prekeys)?;
        self.keys_published = true;
        Ok(())
    }

    fn upload_keys(&mut self, keys: &dyn DeviceKeys, prekeys: deaddrop_crypto::Prekeys) -> Result<(), ClientError> {
        let record = Signed::seal(&self.messenger.keys(now()), |message| keys.sign(message));
        let body = json!({ "keys": record, "one_time": prekeys.one_time, "fallback": prekeys.fallback });
        self.request(keys, Method::PUT, "/v1/keys", Some(body))?;
        Ok(())
    }

    /// Uploads more prekeys when other members have used most of them up.
    fn top_up_prekeys(&mut self, keys: &dyn DeviceKeys, save: Save) -> Result<(), ClientError> {
        let status = self.request(keys, Method::GET, "/v1/keys", None)?;
        let held = status["one_time"].as_u64().unwrap_or(0);
        if held >= PREKEY_LOW {
            return Ok(());
        }
        let prekeys = self.messenger.new_prekeys(PREKEY_TARGET - held as usize, status["fallback"] != true);
        // The new private halves are stored before the public halves are given out.
        save(&self.to_bytes());
        self.upload_keys(keys, prekeys)
    }

    // ------------------------------------------------------------------ running a server

    /// Which ways in the server accepts. Operator only.
    pub fn ways_in(&mut self, keys: &dyn DeviceKeys) -> Result<WaysIn, ClientError> {
        let reply = self.request(keys, Method::GET, "/v1/operator/settings", None)?;
        Ok(WaysIn {
            open: reply["open"] == true,
            tokens: reply["tokens"] == true,
            approval: reply["approval"] == true,
            password: reply["password"] == true,
        })
    }

    /// Switches ways in on and off (`password` is ignored here: see
    /// [`Account::set_server_password`]). Operator only.
    pub fn set_ways_in(&mut self, keys: &dyn DeviceKeys, ways: &WaysIn) -> Result<(), ClientError> {
        let body = json!({ "open": ways.open, "tokens": ways.tokens, "approval": ways.approval });
        self.request(keys, Method::PUT, "/v1/operator/settings", Some(body))?;
        Ok(())
    }

    /// Sets the server password, or removes it with `None`. Operator only.
    pub fn set_server_password(&mut self, keys: &dyn DeviceKeys, password: Option<&str>) -> Result<(), ClientError> {
        self.request(keys, Method::PUT, "/v1/operator/password", Some(json!({ "password": password })))?;
        Ok(())
    }

    /// Devices registered and waiting for approval. Operator only.
    pub fn waiting_devices(&mut self, keys: &dyn DeviceKeys) -> Result<Vec<String>, ClientError> {
        let reply = self.request(keys, Method::GET, "/v1/operator/devices", None)?;
        let devices = reply["devices"].as_array().map(Vec::as_slice).unwrap_or_default();
        Ok(devices
            .iter()
            .filter(|device| device["status"] == "pending")
            .filter_map(|device| device["device_id"].as_str().map(str::to_owned))
            .collect())
    }

    /// Every device registered on the server. Operator only. The server knows devices
    /// by ID alone: names exist only inside channels.
    pub fn devices(&mut self, keys: &dyn DeviceKeys) -> Result<Vec<ServerDevice>, ClientError> {
        let reply = self.request(keys, Method::GET, "/v1/operator/devices", None)?;
        let devices = reply["devices"].as_array().map(Vec::as_slice).unwrap_or_default();
        Ok(devices
            .iter()
            .filter_map(|device| {
                Some(ServerDevice {
                    device: device["device_id"].as_str()?.to_owned(),
                    waiting: device["status"] == "pending",
                    operator: device["operator"] == true,
                })
            })
            .collect())
    }

    /// Approves a waiting device. Operator only.
    pub fn approve_device(&mut self, keys: &dyn DeviceKeys, device: &str) -> Result<(), ClientError> {
        self.request(keys, Method::POST, &format!("/v1/operator/devices/{device}/approve"), None)?;
        Ok(())
    }

    /// Removes a device from the server altogether. Operator only.
    pub fn remove_device(&mut self, keys: &dyn DeviceKeys, device: &str) -> Result<(), ClientError> {
        self.request(keys, Method::DELETE, &format!("/v1/operator/devices/{device}"), None)?;
        Ok(())
    }

    // ------------------------------------------------------------------ channels

    /// Creates a channel owned by this device. Returns its ID.
    pub fn create_channel(
        &mut self,
        keys: &dyn DeviceKeys,
        name: &str,
        my_name: &str,
        max_age: u64,
    ) -> Result<String, ClientError> {
        let id = random_id();
        let record = ChannelRecord {
            v: 1,
            id: id.clone(),
            name: name.to_owned(),
            owner: self.device.clone(),
            max_age,
            version: 1,
            issued: now(),
        };
        let members = vec![RosterMember { device: self.device.clone(), name: my_name.to_owned() }];
        let roster = Roster { v: 1, channel: id.clone(), version: 1, members: members.clone(), issued: now() };
        let body = json!({
            "channel": Signed::seal(&record, |message| keys.sign(message)),
            "roster": Signed::seal(&roster, |message| keys.sign(message)),
        });
        self.request(keys, Method::POST, "/v1/channels", Some(body))?;
        self.messenger.set_roster(&id, 1, std::slice::from_ref(&self.device));
        self.channels.insert(
            id.clone(),
            Channel {
                id: id.clone(),
                name: name.to_owned(),
                owner: self.device.clone(),
                max_age,
                record_version: 1,
                roster_version: 1,
                members,
                left: Vec::new(),
            },
        );
        Ok(id)
    }

    fn owned(&self, channel: &str) -> Result<&Channel, ClientError> {
        let found = self.channels.get(channel).ok_or(ClientError::UnknownChannel)?;
        if found.owner == self.device { Ok(found) } else { Err(ClientError::NotOwner) }
    }

    /// A link that lets its holder ask to join this channel. Owner only.
    pub fn invite_to_channel(
        &mut self,
        keys: &dyn DeviceKeys,
        channel: &str,
        uses: u32,
        ttl_seconds: u64,
    ) -> Result<Link, ClientError> {
        self.owned(channel)?;
        let body = json!({ "uses": uses, "ttl": ttl_seconds });
        let reply = self.request(keys, Method::POST, &format!("/v1/channels/{channel}/invites"), Some(body))?;
        let invite = ChannelInvite {
            channel: channel.to_owned(),
            owner: self.device.clone(),
            token: text(&reply["token"], "invite")?,
        };
        Ok(Link { server: self.address.clone(), server_token: None, channel: Some(invite) })
    }

    /// A link that lets a new device onto the server. Operator only.
    pub fn invite_to_server(
        &mut self,
        keys: &dyn DeviceKeys,
        uses: u32,
        ttl_seconds: u64,
    ) -> Result<Link, ClientError> {
        let body = json!({ "uses": uses, "ttl": ttl_seconds });
        let reply = self.request(keys, Method::POST, "/v1/operator/invites", Some(body))?;
        let token = text(&reply["token"], "invite")?;
        Ok(Link { server: self.address.clone(), server_token: Some(token), channel: None })
    }

    /// Asks to join the channel a link invites to, under `my_name`.
    ///
    /// Before asking, the channel the server shows is checked against the invite: it
    /// must be signed by the owner the invite names.
    pub fn ask_to_join(&mut self, keys: &dyn DeviceKeys, link: &Link, my_name: &str) -> Result<(), ClientError> {
        let invite = link.channel.as_ref().ok_or(ClientError::BadLink)?;
        if link.server.fingerprint != self.address.fingerprint {
            return Err(ClientError::WrongServer);
        }
        self.verified_record(keys, &invite.channel, &invite.owner)?;
        let body = json!({ "token": invite.token, "name": my_name });
        self.request(keys, Method::POST, &format!("/v1/channels/{}/join-requests", invite.channel), Some(body))?;
        self.asked.retain(|asked| asked.channel != invite.channel);
        self.asked.push(Asked { channel: invite.channel.clone(), owner: invite.owner.clone() });
        Ok(())
    }

    /// Who is asking to join a channel this device owns.
    pub fn join_requests(&mut self, keys: &dyn DeviceKeys, channel: &str) -> Result<Vec<JoinRequest>, ClientError> {
        self.owned(channel)?;
        let reply = self.request(keys, Method::GET, &format!("/v1/channels/{channel}/join-requests"), None)?;
        let mut requests = Vec::new();
        for entry in reply["requests"].as_array().map(Vec::as_slice).unwrap_or_default() {
            requests.push(JoinRequest {
                device: text(&entry["device_id"], "join request")?,
                name: text(&entry["name"], "join request")?,
            });
        }
        Ok(requests)
    }

    /// Lets a device that has asked into the channel, by signing a new roster with it in.
    pub fn admit(&mut self, keys: &dyn DeviceKeys, channel: &str, device: &str) -> Result<(), ClientError> {
        let requests = self.join_requests(keys, channel)?;
        let request = requests.into_iter().find(|request| request.device == device);
        let request = request.ok_or(ClientError::Refused { status: 404, message: "no such request".into() })?;
        let mut members = self.owned(channel)?.members.clone();
        members.push(RosterMember { device: request.device, name: request.name });
        self.publish_roster(keys, channel, members)
    }

    /// Removes a member, by signing a new roster without them.
    pub fn remove(&mut self, keys: &dyn DeviceKeys, channel: &str, device: &str) -> Result<(), ClientError> {
        let mut members = self.owned(channel)?.members.clone();
        members.retain(|member| member.device != device);
        self.publish_roster(keys, channel, members)
    }

    fn publish_roster(
        &mut self,
        keys: &dyn DeviceKeys,
        channel: &str,
        mut members: Vec<RosterMember>,
    ) -> Result<(), ClientError> {
        let current = self.owned(channel)?.clone();
        // Anyone who has left is dropped from the signed list at the same time.
        members.retain(|member| !current.left.contains(&member.device));
        let roster =
            Roster { v: 1, channel: channel.to_owned(), version: current.roster_version + 1, members, issued: now() };
        let body = json!({ "roster": Signed::seal(&roster, |message| keys.sign(message)) });
        self.request(keys, Method::PUT, &format!("/v1/channels/{channel}/roster"), Some(body))?;
        self.refresh_channel(keys, channel, &current.owner)?;
        Ok(())
    }

    /// Changes a channel's maximum message age. Owner only.
    pub fn set_max_age(&mut self, keys: &dyn DeviceKeys, channel: &str, max_age: u64) -> Result<(), ClientError> {
        let name = self.owned(channel)?.name.clone();
        self.publish_record(keys, channel, &name, max_age)
    }

    /// Renames a channel. Owner only.
    pub fn rename_channel(&mut self, keys: &dyn DeviceKeys, channel: &str, name: &str) -> Result<(), ClientError> {
        let max_age = self.owned(channel)?.max_age;
        if !valid_name(name, 64) {
            return Err(ClientError::Invalid("a channel's name is 1 to 64 characters"));
        }
        self.publish_record(keys, channel, name, max_age)
    }

    /// The safety number this device shares with another (`keys::safety_number`).
    pub fn safety_number(&self, device: &str) -> String {
        keys::safety_number(&self.device, device)
    }

    /// Changes this device's name in a channel. Returns whether it is done already.
    ///
    /// Names are part of the member list, which only the owner signs. The owner's own
    /// name changes at once. Anyone else's request travels to the owner as a sealed
    /// message, and the owner's phone signs the new list when it next collects.
    pub fn set_my_name(
        &mut self,
        keys: &dyn DeviceKeys,
        channel: &str,
        name: &str,
        save: Save,
    ) -> Result<bool, ClientError> {
        let held = self.channels.get(channel).ok_or(ClientError::UnknownChannel)?;
        name_is_free(held, &self.device, name)?;
        if held.owner == self.device {
            let device = self.device.clone();
            self.rename_member(keys, channel, &device, name)?;
            return Ok(true);
        }
        self.post(keys, channel, json!({ "t": "name", "name": name, "at": now() }), save)?;
        Ok(false)
    }

    /// As the owner: signs a member list with one member's name changed. Does nothing
    /// if the name is not allowed or is someone else's already.
    fn rename_member(
        &mut self,
        keys: &dyn DeviceKeys,
        channel: &str,
        device: &str,
        name: &str,
    ) -> Result<bool, ClientError> {
        let held = self.owned(channel)?;
        if name_is_free(held, device, name).is_err() || held.name_of(device).is_none_or(|current| current == name) {
            return Ok(false);
        }
        let mut members = held.members.clone();
        for member in &mut members {
            if member.device == device {
                member.name = name.to_owned();
            }
        }
        self.publish_roster(keys, channel, members)?;
        Ok(true)
    }

    fn publish_record(
        &mut self,
        keys: &dyn DeviceKeys,
        channel: &str,
        name: &str,
        max_age: u64,
    ) -> Result<(), ClientError> {
        let current = self.owned(channel)?.clone();
        let record = ChannelRecord {
            v: 1,
            id: current.id.clone(),
            name: name.to_owned(),
            owner: current.owner.clone(),
            max_age,
            version: current.record_version + 1,
            issued: now(),
        };
        let body = json!({ "channel": Signed::seal(&record, |message| keys.sign(message)) });
        self.request(keys, Method::PUT, &format!("/v1/channels/{channel}"), Some(body))?;
        self.refresh_channel(keys, channel, &current.owner)?;
        Ok(())
    }

    /// Leaves a channel, forgetting every key held for it.
    pub fn leave(&mut self, keys: &dyn DeviceKeys, channel: &str) -> Result<(), ClientError> {
        if !self.channels.contains_key(channel) {
            return Err(ClientError::UnknownChannel);
        }
        self.request(keys, Method::DELETE, &format!("/v1/channels/{channel}/members/me"), None)?;
        self.forget(channel);
        Ok(())
    }

    /// Deletes a channel for everyone. Owner only.
    pub fn delete_channel(&mut self, keys: &dyn DeviceKeys, channel: &str) -> Result<(), ClientError> {
        self.owned(channel)?;
        self.request(keys, Method::DELETE, &format!("/v1/channels/{channel}"), None)?;
        self.forget(channel);
        Ok(())
    }

    /// Takes this device off the server: its memberships, anything waiting for it, and
    /// every channel it owns are deleted there. The account is of no use afterwards.
    pub fn leave_server(&mut self, keys: &dyn DeviceKeys) -> Result<(), ClientError> {
        self.request(keys, Method::DELETE, "/v1/devices/me", None)?;
        Ok(())
    }

    fn forget(&mut self, channel: &str) {
        self.channels.remove(channel);
        self.messenger.forget_channel(channel);
        self.outbox.retain(|queued| queued.channel != channel);
    }

    // ------------------------------------------------------------------ checking what the server says

    /// Fetches a channel's record and checks it: the owner's key must be the one
    /// `owner` names, and the record must be signed by it and be about this channel.
    fn verified_record(
        &mut self,
        keys: &dyn DeviceKeys,
        channel: &str,
        owner: &str,
    ) -> Result<(ChannelRecord, Vec<u8>), ClientError> {
        let shown = self.request(keys, Method::GET, &format!("/v1/channels/{channel}"), None)?;
        let owner_key = shown["owner_key"].as_str().and_then(unb64).ok_or(ClientError::Untrusted("owner's key"))?;
        if keys::device_id(&owner_key) != owner {
            return Err(ClientError::Untrusted("the channel is not owned by who the invite says"));
        }
        let record: ChannelRecord = signed(&shown["channel"], "channel record")?
            .open(&owner_key)
            .map_err(|_| ClientError::Untrusted("channel record is not signed by its owner"))?;
        if record.id != channel || record.owner != owner {
            return Err(ClientError::Untrusted("channel record is for something else"));
        }
        Ok((record, owner_key))
    }

    /// Brings one channel up to date. Returns whether anything changed.
    ///
    /// Nothing is taken on trust: the record and the roster must be signed by `owner`
    /// and never older than what is already held, and each member's messaging keys must
    /// be signed by the key their device ID in the roster is the hash of.
    fn refresh_channel(&mut self, keys: &dyn DeviceKeys, channel: &str, owner: &str) -> Result<bool, ClientError> {
        let before = self.channels.get(channel).cloned();
        let (record, owner_key) = self.verified_record(keys, channel, owner)?;
        let view = self.request(keys, Method::GET, &format!("/v1/channels/{channel}/members"), None)?;
        let roster: Roster = signed(&view["roster"], "roster")?
            .open(&owner_key)
            .map_err(|_| ClientError::Untrusted("roster is not signed by the channel's owner"))?;
        if roster.channel != channel || !roster.members.iter().any(|member| member.device == self.device) {
            return Err(ClientError::Untrusted("roster is for something else"));
        }
        if let Some(held) = &before
            && (record.version < held.record_version || roster.version < held.roster_version)
        {
            return Err(ClientError::Untrusted("the server offered an older version than one already seen"));
        }
        let mut left: Vec<String> = view["left"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(|id| id.as_str().map(str::to_owned))
            .collect();

        let mut active = Vec::new();
        for member in &roster.members {
            if left.contains(&member.device) {
                continue;
            }
            if member.device == self.device {
                active.push(member.device.clone());
                continue;
            }
            let entries = view["keys"].as_array().map(Vec::as_slice).unwrap_or_default();
            let Some(entry) = entries.iter().find(|entry| entry["device_id"] == member.device.as_str()) else {
                // Their device is gone from the server altogether (they took the server
                // off their phone, or its operator removed them). That counts as having
                // left. A server that hides a member this way gains nothing: it could
                // simply not deliver to them.
                left.push(member.device.clone());
                continue;
            };
            if entry["messaging_keys"].is_null() {
                // Registered but not finished setting up: nothing can be sent to them yet.
                continue;
            }
            let signing_key =
                entry["signing_key"].as_str().and_then(unb64).ok_or(ClientError::Untrusted("a member's key"))?;
            let messaging =
                open_messaging_keys(&member.device, &signing_key, &signed(&entry["messaging_keys"], "keys")?)
                    .map_err(|_| ClientError::Untrusted("a member's keys are not signed by that member"))?;
            self.messenger.add_peer(&messaging).map_err(|_| ClientError::Untrusted("a member's keys have changed"))?;
            active.push(member.device.clone());
        }
        self.messenger.set_roster(channel, roster.version, &active);

        let after = Channel {
            id: channel.to_owned(),
            name: record.name,
            owner: owner.to_owned(),
            max_age: record.max_age,
            record_version: record.version,
            roster_version: roster.version,
            members: roster.members,
            left,
        };
        let changed = before.as_ref() != Some(&after);
        self.channels.insert(channel.to_owned(), after);
        self.asked.retain(|asked| asked.channel != channel);
        Ok(changed)
    }

    /// Brings every channel up to date. Returns whether anything changed.
    fn refresh(&mut self, keys: &dyn DeviceKeys) -> Result<bool, ClientError> {
        let list = self.request(keys, Method::GET, "/v1/channels", None)?;
        let ids = |field: &str| -> Vec<String> {
            let entries = list[field].as_array().map(Vec::as_slice).unwrap_or_default();
            entries.iter().filter_map(|entry| entry["id"].as_str().or(entry.as_str()).map(str::to_owned)).collect()
        };
        let (mine, still_asking) = (ids("channels"), ids("requests"));

        let mut changed = false;
        for channel in &mine {
            // Who should own it: what is already held, or what the invite said.
            let owner = self.channels.get(channel).map(|held| held.owner.clone());
            let owner = owner.or_else(|| self.asked.iter().find(|a| a.channel == *channel).map(|a| a.owner.clone()));
            // A channel nobody invited this device to cannot be checked, so it is ignored.
            let Some(owner) = owner else { continue };
            changed |= self.refresh_channel(keys, channel, &owner)?;
            // The owner's phone then signs a member list without anyone who has left.
            let held = self.channels.get(channel).filter(|held| held.owner == self.device && !held.left.is_empty());
            if let Some(members) = held.map(|held| held.members.clone()) {
                self.publish_roster(keys, channel, members)?;
                changed = true;
            }
        }
        let gone: Vec<String> = self.channels.keys().filter(|id| !mine.contains(id)).cloned().collect();
        for channel in gone {
            // Removed by the owner, or the channel was deleted.
            self.forget(&channel);
            changed = true;
        }
        let asking_before = self.asked.len();
        self.asked.retain(|asked| still_asking.contains(&asked.channel));
        Ok(changed || self.asked.len() != asking_before)
    }

    // ------------------------------------------------------------------ sending

    /// Starts a private session with every member there is none with yet.
    fn connect_members(&mut self, keys: &dyn DeviceKeys, channel: &str) -> Result<(), ClientError> {
        let missing = self.messenger.missing_sessions(channel)?;
        if missing.is_empty() {
            return Ok(());
        }
        let path = format!("/v1/channels/{channel}/keys/claim");
        let reply = self.request(keys, Method::POST, &path, Some(json!({ "devices": missing })))?;
        for claimed in reply["claimed"].as_array().map(Vec::as_slice).unwrap_or_default() {
            let device = text(&claimed["device_id"], "claimed keys")?;
            let prekey: SignedKey = serde_json::from_value(claimed["prekey"].clone())
                .map_err(|_| ClientError::MemberNotReady(device.clone()))?;
            self.messenger
                .start_session(&device, &prekey)
                .map_err(|_| ClientError::Untrusted("a prekey is not signed by the member it is said to be from"))?;
        }
        Ok(())
    }

    /// Sends a text message to a channel. Returns whether it has reached the server;
    /// if the server cannot be reached it stays queued and goes with the next sync.
    ///
    /// `save` is called once the message is sealed and before anything is posted, so a
    /// crash at any point neither loses the message nor reuses a key.
    pub fn send(&mut self, keys: &dyn DeviceKeys, channel: &str, text: &str, save: Save) -> Result<bool, ClientError> {
        self.post(keys, channel, json!({ "t": "text", "body": text, "at": now() }), save)
    }

    /// Seals something for everyone in a channel and posts it.
    fn post(&mut self, keys: &dyn DeviceKeys, channel: &str, payload: Value, save: Save) -> Result<bool, ClientError> {
        if !self.channels.contains_key(channel) {
            return Err(ClientError::UnknownChannel);
        }
        self.connect_members(keys, channel)?;
        let sealed = self.messenger.encrypt(channel, payload.to_string().as_bytes(), now())?;
        for (device, body) in sealed.shares {
            self.outbox.push_back(Outgoing { channel: channel.to_owned(), to: Some(device), body: b64(&body) });
        }
        self.outbox.push_back(Outgoing { channel: channel.to_owned(), to: None, body: b64(&sealed.message) });
        save(&self.to_bytes());
        self.flush(keys, save)
    }

    /// Posts whatever is waiting to go. Returns whether the outbox is now empty.
    fn flush(&mut self, keys: &dyn DeviceKeys, save: Save) -> Result<bool, ClientError> {
        let mut posted = false;
        while let Some(next) = self.outbox.front().cloned() {
            let mut body = json!({ "body": next.body });
            if let Some(device) = &next.to {
                body["to"] = json!([device]);
            }
            match self.request(keys, Method::POST, &format!("/v1/channels/{}/messages", next.channel), Some(body)) {
                Ok(_) => {}
                Err(ClientError::Unreachable) => break,
                Err(ClientError::Refused { .. }) => {
                    // The server will never take this one (the member has gone, or this
                    // device has). If it was a key hand-over, start a new ratchet so the
                    // next message hands keys over afresh.
                    if next.to.is_some() {
                        self.messenger.restart_sending(&next.channel);
                    }
                }
                Err(other) => return Err(other),
            }
            self.outbox.pop_front();
            posted = true;
        }
        if posted {
            save(&self.to_bytes());
        }
        Ok(self.outbox.is_empty())
    }

    // ------------------------------------------------------------------ receiving

    /// Does everything due: finishes setting up, posts anything queued, brings channels
    /// up to date, and collects and opens the queue.
    ///
    /// `save` is called before each batch of messages is acknowledged, because opening
    /// a message destroys the keys for it: the new state must be stored before the
    /// server is told it may wipe its copy.
    pub fn sync(&mut self, keys: &dyn DeviceKeys, save: Save) -> Result<Vec<Event>, ClientError> {
        if !self.keys_published {
            self.publish_keys(keys)?;
            save(&self.to_bytes());
        }
        self.flush(keys, save)?;
        let mut events = Vec::new();
        if self.refresh(keys)? {
            events.push(Event::ChannelsChanged);
        }
        // Members whose keys for a channel this device turned out not to have.
        let mut lost: Vec<(String, String)> = Vec::new();

        loop {
            let reply = self.request(keys, Method::GET, "/v1/queue", None)?;
            let waiting = reply["messages"].as_array().cloned().unwrap_or_default();
            if waiting.is_empty() {
                break;
            }
            let mut refreshed = false;
            let mut ids = Vec::new();
            let mut renames = Vec::new();
            for message in &waiting {
                let (Some(id), Some(channel), Some(sender), Some(body)) = (
                    message["id"].as_str(),
                    message["channel"].as_str(),
                    message["sender"].as_str(),
                    message["body"].as_str().and_then(unb64),
                ) else {
                    continue;
                };
                ids.push(id.to_owned());
                let mut opened = self.messenger.decrypt(channel, sender, &body);
                // This device may simply be behind on who is in the channel.
                let behind = matches!(
                    opened,
                    Err(CryptoError::UnknownPeer(_) | CryptoError::NotAMember | CryptoError::UnknownChannel)
                );
                if behind && !refreshed {
                    refreshed = true;
                    if self.refresh(keys)? {
                        events.push(Event::ChannelsChanged);
                    }
                    opened = self.messenger.decrypt(channel, sender, &body);
                }
                match opened {
                    Ok(Incoming::Message(plaintext)) => {
                        let Ok(payload) = serde_json::from_slice::<Value>(&plaintext) else { continue };
                        // Kinds of message this version does not know are skipped.
                        match payload["t"].as_str() {
                            Some("text") => events.push(Event::Message {
                                channel: channel.to_owned(),
                                sender: sender.to_owned(),
                                sender_name: self
                                    .channels
                                    .get(channel)
                                    .and_then(|c| c.name_of(sender))
                                    .map(str::to_owned),
                                text: payload["body"].as_str().unwrap_or_default().to_owned(),
                                sent: payload["at"].as_i64().unwrap_or(0),
                            }),
                            // A member asking the owner for a new name in the channel.
                            Some("name") => {
                                if let Some(name) = payload["name"].as_str() {
                                    renames.push((channel.to_owned(), sender.to_owned(), name.to_owned()));
                                }
                            }
                            _ => {}
                        }
                    }
                    Ok(Incoming::KeysReceived | Incoming::KeysWanted) => {}
                    // Delivered twice (acknowledging was interrupted last time): already shown.
                    Err(CryptoError::AlreadyOpened) => {}
                    Err(error) => {
                        // The sender's keys never arrived, or the private session they came
                        // over is out of step: ask for them again.
                        let keys_lost = matches!(error, CryptoError::UnknownSession)
                            || (body.first() == Some(&1) && matches!(error, CryptoError::CannotOpen));
                        if keys_lost {
                            lost.push((channel.to_owned(), sender.to_owned()));
                        }
                        // Only a message that cannot be read is worth telling the person about.
                        if body.first() == Some(&2) {
                            events.push(Event::Unreadable { channel: channel.to_owned(), sender: sender.to_owned() });
                        }
                    }
                }
            }
            save(&self.to_bytes());
            self.request(keys, Method::POST, "/v1/queue/ack", Some(json!({ "ids": ids })))?;
            for (channel, device, name) in renames {
                // Only the owner's phone acts on these; a refusal is not worth stopping for.
                match self.rename_member(keys, &channel, &device, &name) {
                    Ok(true) => events.push(Event::ChannelsChanged),
                    Ok(false)
                    | Err(ClientError::NotOwner | ClientError::UnknownChannel | ClientError::Refused { .. }) => {}
                    Err(other) => return Err(other),
                }
            }
            if ids.is_empty() {
                break;
            }
        }
        lost.sort();
        lost.dedup();
        for (channel, sender) in lost {
            self.ask_for_keys(keys, &channel, &sender)?;
        }
        if !self.outbox.is_empty() {
            save(&self.to_bytes());
            self.flush(keys, save)?;
        }
        self.top_up_prekeys(keys, save)?;
        Ok(events)
    }

    /// Asks a member to hand over their keys for a channel again, over a new private
    /// session, so that their later messages can be read. At most once a minute for
    /// each member and channel. What was already missed stays missed.
    fn ask_for_keys(&mut self, keys: &dyn DeviceKeys, channel: &str, sender: &str) -> Result<(), ClientError> {
        let (now, asked) = (now(), (channel.to_owned(), sender.to_owned()));
        if self.asked_keys.get(&asked).is_some_and(|at| now - at < ASK_AGAIN_SECONDS) {
            return Ok(());
        }
        let member = self.channels.get(channel).is_some_and(|held| held.name_of(sender).is_some());
        if !member || !self.messenger.knows_peer(sender) {
            return Ok(());
        }
        let path = format!("/v1/channels/{channel}/keys/claim");
        let reply = match self.request(keys, Method::POST, &path, Some(json!({ "devices": [sender] }))) {
            Ok(reply) => reply,
            Err(ClientError::Refused { .. }) => return Ok(()),
            Err(other) => return Err(other),
        };
        let Ok(prekey) = serde_json::from_value::<SignedKey>(reply["claimed"][0]["prekey"].clone()) else {
            return Ok(());
        };
        if self.messenger.start_session(sender, &prekey).is_err() {
            return Ok(());
        }
        let Ok(body) = self.messenger.ask_again(channel, sender) else { return Ok(()) };
        self.outbox.push_back(Outgoing { channel: channel.to_owned(), to: Some(sender.to_owned()), body: b64(&body) });
        self.asked_keys.insert(asked, now);
        Ok(())
    }

    /// Opens the connection over which the server says when to [`Account::sync`].
    pub fn open_live(&mut self, keys: &dyn DeviceKeys) -> Result<Live, ClientError> {
        let token = self.token(keys)?;
        Live::connect(&self.address, &token)
    }

    // ------------------------------------------------------------------ keeping state

    /// Everything this account holds. **Secret key material**: encrypt it before it
    /// touches a disk.
    pub fn to_bytes(&self) -> Vec<u8> {
        let saved = Saved {
            v: 1,
            address: self.address.clone(),
            device: self.device.clone(),
            server_id: self.server_id.clone(),
            operator: self.operator,
            keys_published: self.keys_published,
            messenger: b64(&self.messenger.to_bytes()),
            channels: self.channels.values().cloned().collect(),
            asked: self.asked.clone(),
            outbox: self.outbox.iter().cloned().collect(),
        };
        serde_json::to_vec(&saved).expect("plain data")
    }

    /// Restores an account from [`Account::to_bytes`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Account, ClientError> {
        let saved: Saved = serde_json::from_slice(bytes).map_err(|_| ClientError::BadState)?;
        if saved.v != 1 {
            return Err(ClientError::BadState);
        }
        let messenger = unb64(&saved.messenger).ok_or(ClientError::BadState)?;
        Ok(Account {
            api: Api::new(&saved.address)?,
            address: saved.address,
            device: saved.device,
            server_id: saved.server_id,
            operator: saved.operator,
            keys_published: saved.keys_published,
            session: None,
            messenger: Messenger::from_bytes(&messenger).map_err(|_| ClientError::BadState)?,
            channels: saved.channels.into_iter().map(|channel| (channel.id.clone(), channel)).collect(),
            asked: saved.asked,
            outbox: saved.outbox.into_iter().collect(),
            asked_keys: HashMap::new(),
        })
    }
}
