//! The bridge between the Android app (Kotlin) and the client engine (Rust).
//!
//! This crate adds no behaviour. It restates `deaddrop_client::Account` in the shapes
//! UniFFI can carry across to Kotlin, and takes two things back from Kotlin: the
//! device's hardware keys ([`KeyHolder`]) and somewhere safe to keep state
//! ([`StateStore`]).
//!
//! Every call blocks while it talks to the server: call from a background thread.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use deaddrop_client::{Account, ClientError, DeviceKeys, Event, Link, Live, Wake, WayIn, WaysIn};

uniffi::setup_scaffolding!();

/// The device's two long-term keys, held in the Android Keystore. The private halves
/// never cross this bridge: the engine only ever asks for signatures.
#[uniffi::export(with_foreign)]
pub trait KeyHolder: Send + Sync {
    /// The signing public key, DER SubjectPublicKeyInfo (P-256).
    fn signing_key_der(&self) -> Vec<u8>;
    /// The agreement public key, DER SubjectPublicKeyInfo (P-256).
    fn agreement_key_der(&self) -> Vec<u8>;
    /// The signing key's signature over the agreement key message, base64.
    fn agreement_signature(&self) -> String;
    /// A DER ECDSA signature over SHA-256(`message`), made with the signing key.
    fn sign(&self, message: Vec<u8>) -> Vec<u8>;
}

/// Where the engine's state is kept. `save` is called at the moments the state must
/// be stored before the engine goes on, so it must not return until the bytes are
/// safely written. They are secret key material: encrypt them with a Keystore key.
#[uniffi::export(with_foreign)]
pub trait StateStore: Send + Sync {
    fn save(&self, state: Vec<u8>);
}

struct Keys(Arc<dyn KeyHolder>);

impl DeviceKeys for Keys {
    fn signing_key_der(&self) -> Vec<u8> {
        self.0.signing_key_der()
    }
    fn agreement_key_der(&self) -> Vec<u8> {
        self.0.agreement_key_der()
    }
    fn agreement_signature(&self) -> String {
        self.0.agreement_signature()
    }
    fn sign(&self, message: &[u8]) -> Vec<u8> {
        self.0.sign(message.to_vec())
    }
}

// Carried to Kotlin as an exception per variant, each with one message.
#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum EngineError {
    /// Not a complete `deaddrop://` link.
    #[error("{message}")]
    BadLink { message: String },
    /// The server cannot be reached, or what answered is not the server with the
    /// expected fingerprint.
    #[error("{message}")]
    Unreachable { message: String },
    /// The server said no.
    #[error("{message}")]
    Refused { status: u16, message: String },
    /// The operator has not approved this device yet.
    #[error("{message}")]
    AwaitingApproval { message: String },
    /// The server presented something that fails a check.
    #[error("{message}")]
    Untrusted { message: String },
    #[error("{message}")]
    Other { message: String },
}

impl From<ClientError> for EngineError {
    fn from(error: ClientError) -> EngineError {
        let message = error.to_string();
        match error {
            ClientError::BadLink => EngineError::BadLink { message },
            ClientError::Unreachable => EngineError::Unreachable { message },
            ClientError::Refused { status, message } => EngineError::Refused { status, message },
            ClientError::AwaitingApproval => EngineError::AwaitingApproval { message },
            ClientError::Untrusted(_) => EngineError::Untrusted { message },
            _ => EngineError::Other { message },
        }
    }
}

type Outcome<T> = Result<T, EngineError>;

/// What a `deaddrop://` link holds, for the app to show before acting on it.
#[derive(uniffi::Record)]
pub struct LinkInfo {
    pub host: String,
    pub port: u16,
    /// The fingerprint in groups of four, for people to read.
    pub fingerprint: String,
    /// Whether the link lets a new device onto the server.
    pub has_server_token: bool,
    /// The channel the link invites to, if it is a channel invite.
    pub channel: Option<String>,
}

/// Reads a link without contacting anything. `None` if it is not a complete link.
#[uniffi::export]
pub fn read_link(text: String) -> Option<LinkInfo> {
    let link = Link::parse(&text)?;
    Some(LinkInfo {
        host: link.server.host,
        port: link.server.port,
        fingerprint: deaddrop_tls::display(&link.server.fingerprint),
        has_server_token: link.server_token.is_some(),
        channel: link.channel.map(|invite| invite.channel),
    })
}

/// Builds a link from an address typed by hand and a fingerprint the person has
/// compared. `None` if the fingerprint is not a fingerprint.
#[uniffi::export]
pub fn link_from_parts(host: String, port: u16, fingerprint: String, token: Option<String>) -> Option<String> {
    let fingerprint = deaddrop_tls::parse(&fingerprint)?;
    let token = token.map(|token| format!("&t={token}")).unwrap_or_default();
    let text = format!("deaddrop://{host}:{port}/#fp={fingerprint}{token}");
    Link::parse(&text).map(|link| link.to_string())
}

/// Asks whatever answers at an address typed by hand for its fingerprint (in groups of
/// four), for the person to compare with what the server's operator gave them. It
/// proves nothing until they have: see `deaddrop_tls::ask_fingerprint`.
#[uniffi::export]
pub fn ask_fingerprint(host: String, port: u16) -> Outcome<String> {
    let found = deaddrop_tls::ask_fingerprint(&host, port, Duration::from_secs(10));
    Ok(deaddrop_tls::display(&found.ok_or(ClientError::Unreachable)?))
}

/// A device on a server, as the person who runs it sees it.
#[derive(uniffi::Record)]
pub struct DeviceInfo {
    pub device: String,
    /// Registered but not yet approved.
    pub waiting: bool,
    pub operator: bool,
}

#[derive(uniffi::Record)]
pub struct Member {
    pub device: String,
    pub name: String,
    /// Has left since the owner last signed the member list.
    pub has_left: bool,
}

#[derive(uniffi::Record)]
pub struct ChannelInfo {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub owned_by_me: bool,
    /// Maximum message age, in seconds.
    pub max_age: u64,
    pub members: Vec<Member>,
}

#[derive(uniffi::Record)]
pub struct JoinRequestInfo {
    pub device: String,
    pub name: String,
}

#[derive(uniffi::Record)]
pub struct WaysInInfo {
    pub open: bool,
    pub tokens: bool,
    pub approval: bool,
    pub password_set: bool,
}

/// A message carries `id`, a random identifier the engine gives it. The engine hands a
/// message over, under the same ID, until [`Session::stored`] says the app has it.
#[derive(uniffi::Enum)]
pub enum News {
    Message {
        id: String,
        channel: String,
        sender: String,
        sender_name: Option<String>,
        text: String,
        sent: i64,
    },
    /// A message arrived that cannot be opened.
    Unreadable {
        id: String,
        channel: String,
        sender: String,
    },
    /// The channel list, or something about a channel, changed.
    ChannelsChanged,
}

impl From<Event> for News {
    fn from(event: Event) -> News {
        match event {
            Event::Message { id, channel, sender, sender_name, text, sent } => {
                News::Message { id, channel, sender, sender_name, text, sent }
            }
            Event::Unreadable { id, channel, sender } => News::Unreadable { id, channel, sender },
            Event::ChannelsChanged => News::ChannelsChanged,
        }
    }
}

/// One identity on one server.
#[derive(uniffi::Object)]
pub struct Session {
    account: Mutex<Account>,
    live: Mutex<Option<Live>>,
    keys: Keys,
    store: Arc<dyn StateStore>,
}

impl Session {
    fn new(account: Account, keys: Arc<dyn KeyHolder>, store: Arc<dyn StateStore>) -> Arc<Session> {
        Arc::new(Session { account: Mutex::new(account), live: Mutex::new(None), keys: Keys(keys), store })
    }

    fn account(&self) -> MutexGuard<'_, Account> {
        self.account.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Runs something that changes the account, then stores the result.
    fn change<T>(&self, work: impl FnOnce(&mut Account, &Keys) -> Result<T, ClientError>) -> Outcome<T> {
        let mut account = self.account();
        let result = work(&mut account, &self.keys);
        // Stored even after a failure: the attempt may have moved keys on.
        self.store.save(account.to_bytes());
        Ok(result?)
    }
}

#[uniffi::export]
impl Session {
    /// Registers this device on the server a link points to.
    #[uniffi::constructor]
    pub fn join(
        link: String,
        password: Option<String>,
        keys: Arc<dyn KeyHolder>,
        store: Arc<dyn StateStore>,
    ) -> Outcome<Arc<Session>> {
        let link = Link::parse(&link).ok_or(ClientError::BadLink)?;
        let way_in = WayIn { token: link.server_token.clone(), password };
        let account = Account::join(&link.server, &way_in, &Keys(keys.clone()))?;
        store.save(account.to_bytes());
        Ok(Session::new(account, keys, store))
    }

    /// Restores a session from state the [`StateStore`] was given earlier.
    #[uniffi::constructor]
    pub fn restore(state: Vec<u8>, keys: Arc<dyn KeyHolder>, store: Arc<dyn StateStore>) -> Outcome<Arc<Session>> {
        Ok(Session::new(Account::from_bytes(&state)?, keys, store))
    }

    pub fn device(&self) -> String {
        self.account().device().to_owned()
    }

    /// The server's address, as `host:port`.
    pub fn address(&self) -> String {
        self.account().address().authority()
    }

    /// The server's fingerprint in groups of four.
    pub fn fingerprint(&self) -> String {
        deaddrop_tls::display(&self.account().address().fingerprint)
    }

    pub fn is_operator(&self) -> bool {
        self.account().operator()
    }

    /// False while the device is waiting for the operator's approval.
    pub fn is_approved(&self) -> bool {
        self.account().approved()
    }

    pub fn channels(&self) -> Vec<ChannelInfo> {
        let account = self.account();
        account
            .channels()
            .map(|channel| ChannelInfo {
                id: channel.id.clone(),
                name: channel.name.clone(),
                owner: channel.owner.clone(),
                owned_by_me: channel.owner == account.device(),
                max_age: channel.max_age,
                members: channel
                    .members
                    .iter()
                    .map(|member| Member {
                        device: member.device.clone(),
                        name: member.name.clone(),
                        has_left: channel.left.contains(&member.device),
                    })
                    .collect(),
            })
            .collect()
    }

    /// A link holding only this server's address and fingerprint: enough to add the
    /// server where the way in is a password, approval, or nothing.
    pub fn server_link(&self) -> String {
        let server = self.account().address().clone();
        Link { server, server_token: None, channel: None }.to_string()
    }

    /// The safety number this device shares with another: thirty digits in six groups.
    pub fn safety_number(&self, device: String) -> String {
        self.account().safety_number(&device)
    }

    /// Channels this device has asked to join and is waiting to be let into.
    pub fn asked(&self) -> Vec<String> {
        self.account().asked().map(str::to_owned).collect()
    }

    /// Sealed messages waiting for the server to be reachable.
    pub fn unsent(&self) -> u32 {
        self.account().unsent() as u32
    }

    pub fn create_channel(&self, name: String, my_name: String, max_age: u64) -> Outcome<String> {
        self.change(|account, keys| account.create_channel(keys, &name, &my_name, max_age))
    }

    /// A link that lets one person ask to join, good for a day.
    pub fn invite_to_channel(&self, channel: String) -> Outcome<String> {
        self.change(|account, keys| account.invite_to_channel(keys, &channel, 1, 86_400)).map(|link| link.to_string())
    }

    /// A link that lets one new device onto the server, good for a day. Operator only.
    pub fn invite_to_server(&self) -> Outcome<String> {
        self.change(|account, keys| account.invite_to_server(keys, 1, 86_400)).map(|link| link.to_string())
    }

    pub fn ask_to_join(&self, link: String, my_name: String) -> Outcome<()> {
        let link = Link::parse(&link).ok_or(ClientError::BadLink)?;
        self.change(|account, keys| account.ask_to_join(keys, &link, &my_name))
    }

    pub fn join_requests(&self, channel: String) -> Outcome<Vec<JoinRequestInfo>> {
        let requests = self.change(|account, keys| account.join_requests(keys, &channel))?;
        Ok(requests.into_iter().map(|request| JoinRequestInfo { device: request.device, name: request.name }).collect())
    }

    pub fn admit(&self, channel: String, device: String) -> Outcome<()> {
        self.change(|account, keys| account.admit(keys, &channel, &device))
    }

    pub fn remove_member(&self, channel: String, device: String) -> Outcome<()> {
        self.change(|account, keys| account.remove(keys, &channel, &device))
    }

    pub fn set_max_age(&self, channel: String, max_age: u64) -> Outcome<()> {
        self.change(|account, keys| account.set_max_age(keys, &channel, max_age))
    }

    pub fn leave(&self, channel: String) -> Outcome<()> {
        self.change(|account, keys| account.leave(keys, &channel))
    }

    pub fn delete_channel(&self, channel: String) -> Outcome<()> {
        self.change(|account, keys| account.delete_channel(keys, &channel))
    }

    /// Renames a channel. Owner only.
    pub fn rename_channel(&self, channel: String, name: String) -> Outcome<()> {
        self.change(|account, keys| account.rename_channel(keys, &channel, &name))
    }

    /// Changes this device's name in a channel. True if it is done already; false if
    /// the request has gone to the channel's owner, whose phone makes the change.
    pub fn set_my_name(&self, channel: String, name: String) -> Outcome<bool> {
        let mut account = self.account();
        let store = &self.store;
        let done = account.set_my_name(&self.keys, &channel, &name, &mut |bytes| store.save(bytes.to_vec()));
        store.save(account.to_bytes());
        Ok(done?)
    }

    /// Takes this device off the server, with its memberships, anything waiting for it
    /// and every channel it owns. The session is of no use afterwards.
    pub fn leave_server(&self) -> Outcome<()> {
        self.change(|account, keys| account.leave_server(keys))
    }

    /// Sends a text message. False means it is sealed and stored but the server could
    /// not be reached; it goes with the next [`Session::sync`].
    pub fn send(&self, channel: String, text: String) -> Outcome<bool> {
        let mut account = self.account();
        let store = &self.store;
        let sent = account.send(&self.keys, &channel, &text, &mut |bytes| store.save(bytes.to_vec()));
        store.save(account.to_bytes());
        Ok(sent?)
    }

    /// Does everything due and returns what is new, with every message not yet
    /// [`Session::stored`]. Store the messages, then call that.
    ///
    /// After an error, call [`Session::received`]: messages may have been opened before
    /// whatever failed, and the engine is by then the only place that holds them.
    pub fn sync(&self) -> Outcome<Vec<News>> {
        let mut account = self.account();
        let store = &self.store;
        let events = account.sync(&self.keys, &mut |bytes| store.save(bytes.to_vec()));
        store.save(account.to_bytes());
        Ok(events?.into_iter().map(News::from).collect())
    }

    /// Every message the engine has opened and the app has not yet [`Session::stored`].
    pub fn received(&self) -> Vec<News> {
        self.account().received().into_iter().map(News::from).collect()
    }

    /// The app has stored the messages with these IDs, somewhere that survives a
    /// crash, so the engine may forget them.
    pub fn stored(&self, ids: Vec<String>) {
        let mut account = self.account();
        account.stored(&ids);
        self.store.save(account.to_bytes());
    }

    /// Waits up to `seconds` for the server to say there is something to collect.
    /// True means call [`Session::sync`] now. An error means the connection could not
    /// be made or has dropped: wait a little and call again.
    pub fn wait_for_news(&self, seconds: u32) -> Outcome<bool> {
        let mut live = self.live.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if live.is_none() {
            *live = Some(self.account().open_live(&self.keys)?);
        }
        match live.as_mut().expect("just opened").wait(Duration::from_secs(u64::from(seconds))) {
            Ok(wake) => Ok(wake == Wake::Sync),
            Err(error) => {
                *live = None;
                Err(error.into())
            }
        }
    }

    // ---- for the device that runs the server

    pub fn ways_in(&self) -> Outcome<WaysInInfo> {
        let ways = self.change(|account, keys| account.ways_in(keys))?;
        Ok(WaysInInfo { open: ways.open, tokens: ways.tokens, approval: ways.approval, password_set: ways.password })
    }

    pub fn set_ways_in(&self, open: bool, tokens: bool, approval: bool) -> Outcome<()> {
        let ways = WaysIn { open, tokens, approval, password: false };
        self.change(|account, keys| account.set_ways_in(keys, &ways))
    }

    /// Sets the server password, or removes it with null.
    pub fn set_server_password(&self, password: Option<String>) -> Outcome<()> {
        self.change(|account, keys| account.set_server_password(keys, password.as_deref()))
    }

    pub fn waiting_devices(&self) -> Outcome<Vec<String>> {
        self.change(|account, keys| account.waiting_devices(keys))
    }

    pub fn approve_device(&self, device: String) -> Outcome<()> {
        self.change(|account, keys| account.approve_device(keys, &device))
    }

    /// Every device on the server.
    pub fn devices(&self) -> Outcome<Vec<DeviceInfo>> {
        let devices = self.change(|account, keys| account.devices(keys))?;
        Ok(devices
            .into_iter()
            .map(|device| DeviceInfo { device: device.device, waiting: device.waiting, operator: device.operator })
            .collect())
    }

    /// Removes a device from the server altogether, with the channels it owns.
    pub fn remove_device(&self, device: String) -> Outcome<()> {
        self.change(|account, keys| account.remove_device(keys, &device))
    }
}
