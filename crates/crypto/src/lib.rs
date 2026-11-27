//! End-to-end encryption for Dead Drop channels (docs/PROTOCOL.md section 6).
//!
//! A [`Messenger`] is everything one device knows on one server: its own messaging
//! keys, a private session with each device it has exchanged keys with, and for every
//! channel one sending ratchet of its own plus a receiving ratchet per other member.
//! It never touches a network or a disk. The caller moves the bytes it produces and
//! stores [`Messenger::to_bytes`] somewhere encrypted.
//!
//! The cryptography itself is [vodozemac](https://github.com/matrix-org/vodozemac),
//! the Matrix project's implementation of the Double Ratchet ("Olm", between two
//! devices) and of per-sender group ratchets ("Megolm"). This crate adds what is
//! particular to Dead Drop: which keys are trusted (those the channel owner's signed
//! roster leads to), when sending ratchets are replaced, and that receiving ratchets
//! are wound forward after every message so that old messages cannot be opened again.

mod padding;
mod saved;

use std::collections::{BTreeSet, HashMap};

use deaddrop_core::records::{MessagingKeys, SignedKey, prekey_message};
use serde::{Deserialize, Serialize};
use vodozemac::megolm::{
    DecryptionError as GroupDecryptionError, GroupSession, InboundGroupSession, MegolmMessage, SessionKey,
};
use vodozemac::olm::{Account, OlmMessage, Session};
use vodozemac::{Curve25519PublicKey, Ed25519PublicKey, Ed25519Signature};

pub use padding::MAX_PLAINTEXT_BYTES;

/// A sending ratchet is replaced after this many messages...
pub const ROTATE_AFTER_MESSAGES: u32 = 100;
/// ...or after this long, whichever comes first, as well as whenever the roster
/// changes. A copied ratchet can read only until its owner replaces it, so this bounds
/// how much a one-off leak of one ratchet gives away.
pub const ROTATE_AFTER_SECONDS: i64 = 7 * 86_400;

/// Private sessions kept per other device. More than one can exist when two devices
/// start talking to each other at the same moment.
const MAX_SESSIONS_PER_PEER: usize = 4;
/// Receiving ratchets kept per sender per channel: the current one and a few before it,
/// for messages still on their way when the sender replaced theirs.
const MAX_INBOUND_PER_SENDER: usize = 4;
/// The most prekeys worth asking for at once.
pub const MAX_PREKEYS: usize = 50;

const PAIRWISE: u8 = 1;
const CHANNEL: u8 = 2;

fn olm_config() -> vodozemac::olm::SessionConfig {
    vodozemac::olm::SessionConfig::version_1()
}

fn megolm_config() -> vodozemac::megolm::SessionConfig {
    vodozemac::megolm::SessionConfig::version_1()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The channel has not been given a roster with [`Messenger::set_roster`].
    UnknownChannel,
    /// The device named is not in the roster this messenger holds for the channel.
    /// If the roster might be out of date, fetch it and try again.
    NotAMember,
    /// No messaging keys are known for this device: fetch them, check them, and call
    /// [`Messenger::add_peer`].
    UnknownPeer(String),
    /// A device's messaging keys are not the ones seen before.
    KeysChanged,
    /// Keys that do not parse, or a prekey not signed by the device it is said to be from.
    BadKeys,
    /// There is no private session yet with these members: claim a prekey for each
    /// and call [`Messenger::start_session`], then try again.
    NoSession(Vec<String>),
    /// The plaintext is longer than [`MAX_PLAINTEXT_BYTES`].
    TooLarge,
    /// Not something this crate produced.
    Malformed,
    /// A message for a sending ratchet this device was never given (its key may have
    /// expired from the queue before it was collected).
    UnknownSession,
    /// The message was already opened, or is older than one that was. Keys for it no
    /// longer exist.
    AlreadyOpened,
    /// The message does not open: tampered with, or not from the claimed sender.
    CannotOpen,
}

/// What [`Messenger::encrypt`] produces. Send every share first, each to the one
/// device named, then `message` to the whole channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub shares: Vec<(String, Vec<u8>)>,
    pub message: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    /// A message from a member of the channel.
    Message(Vec<u8>),
    /// Keys from another member arrived and were stored; nothing to show.
    KeysReceived,
    /// Another member asked for this device's keys for the channel again
    /// ([`Messenger::ask_again`]). They go with the next message; nothing to show.
    KeysWanted,
}

/// Prekeys to upload, made by [`Messenger::new_prekeys`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prekeys {
    pub one_time: Vec<SignedKey>,
    /// A prekey the server hands out when the one-time ones have run out.
    pub fallback: Option<SignedKey>,
}

struct Peer {
    curve25519: Curve25519PublicKey,
    ed25519: Ed25519PublicKey,
    /// Most recently used last.
    sessions: Vec<Session>,
}

struct Outbound {
    session: GroupSession,
    roster_version: u64,
    created: i64,
    shared_with: BTreeSet<String>,
}

#[derive(Default)]
struct Channel {
    roster_version: u64,
    members: BTreeSet<String>,
    outbound: Option<Outbound>,
    /// Per sender, newest last.
    inbound: HashMap<String, Vec<InboundGroupSession>>,
}

/// What travels inside a private session between two devices.
#[derive(Serialize, Deserialize)]
struct Pairwise {
    /// Naming both ends inside the encryption stops a message being passed off as
    /// meant for, or coming from, some other device.
    from: String,
    to: String,
    channel: String,
    /// The roster version the sender held when it made this ratchet.
    roster: u64,
    /// The sender's sending ratchet for the channel, at its current position.
    key: String,
    /// Set (with no key) to ask the other device for *its* sending ratchet again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    ask: bool,
}

pub struct Messenger {
    device: String,
    account: Account,
    peers: HashMap<String, Peer>,
    channels: HashMap<String, Channel>,
}

impl Messenger {
    /// A new set of messaging keys for the device with this ID.
    pub fn new(device: &str) -> Messenger {
        Messenger {
            device: device.to_owned(),
            account: Account::new(),
            peers: HashMap::new(),
            channels: HashMap::new(),
        }
    }

    pub fn device(&self) -> &str {
        &self.device
    }

    /// This device's messaging keys, to be signed with the device's signing key
    /// (`deaddrop_core::records::Signed::seal`) and published.
    pub fn keys(&self, issued: i64) -> MessagingKeys {
        MessagingKeys {
            v: 1,
            device: self.device.clone(),
            curve25519: self.account.curve25519_key().to_base64(),
            ed25519: self.account.ed25519_key().to_base64(),
            issued,
        }
    }

    /// Makes `count` new one-time prekeys (at most [`MAX_PREKEYS`]), and a new fallback
    /// prekey if asked, signed and ready to upload.
    pub fn new_prekeys(&mut self, count: usize, new_fallback: bool) -> Prekeys {
        self.account.generate_one_time_keys(count.min(MAX_PREKEYS));
        if new_fallback {
            self.account.generate_fallback_key();
        }
        let sign = |key: Curve25519PublicKey| {
            let key = key.to_base64();
            let signature = self.account.sign(prekey_message(&self.device, &key)).to_base64();
            SignedKey { key, signature }
        };
        let one_time = self.account.one_time_keys().into_values().map(sign).collect();
        let fallback = self.account.fallback_key().into_values().next().map(sign);
        self.account.mark_keys_as_published();
        Prekeys { one_time, fallback }
    }

    /// Records another device's messaging keys.
    ///
    /// The caller must already have checked that `keys` was signed by that device's
    /// signing key, and that the signing key hashes to the device ID in the owner-signed
    /// roster. That chain is what makes these keys trustworthy.
    pub fn add_peer(&mut self, keys: &MessagingKeys) -> Result<(), Error> {
        let curve25519 = Curve25519PublicKey::from_base64(&keys.curve25519).map_err(|_| Error::BadKeys)?;
        let ed25519 = Ed25519PublicKey::from_base64(&keys.ed25519).map_err(|_| Error::BadKeys)?;
        if keys.device == self.device {
            return Err(Error::BadKeys);
        }
        match self.peers.get(&keys.device) {
            Some(peer) if peer.curve25519 == curve25519 && peer.ed25519 == ed25519 => Ok(()),
            Some(_) => Err(Error::KeysChanged),
            None => {
                self.peers.insert(keys.device.clone(), Peer { curve25519, ed25519, sessions: Vec::new() });
                Ok(())
            }
        }
    }

    pub fn knows_peer(&self, device: &str) -> bool {
        self.peers.contains_key(device)
    }

    /// Sets who is in a channel, from a roster whose owner signature the caller has
    /// checked. An older version than the one held is ignored.
    ///
    /// Receiving ratchets from devices no longer listed are destroyed, and this device's
    /// own sending ratchet is replaced before its next message, so that nobody who was
    /// removed can read on.
    pub fn set_roster(&mut self, channel: &str, version: u64, members: &[String]) {
        let state = self.channels.entry(channel.to_owned()).or_default();
        if version < state.roster_version {
            return;
        }
        state.roster_version = version;
        state.members = members.iter().cloned().collect();
        state.inbound.retain(|sender, _| state.members.contains(sender));
    }

    /// Forgets a channel and every key held for it.
    pub fn forget_channel(&mut self, channel: &str) {
        self.channels.remove(channel);
    }

    /// Throws away this device's sending ratchet for a channel, so that the next message
    /// starts a new one and hands it to every member again. For when a hand-over could
    /// not be sent: the members it was meant for would otherwise never get it.
    pub fn restart_sending(&mut self, channel: &str) {
        if let Some(state) = self.channels.get_mut(channel) {
            state.outbound = None;
        }
    }

    /// The other members of a channel this device cannot yet send keys to, because it
    /// has no private session with them.
    pub fn missing_sessions(&self, channel: &str) -> Result<Vec<String>, Error> {
        let state = self.channels.get(channel).ok_or(Error::UnknownChannel)?;
        Ok(state
            .members
            .iter()
            .filter(|member| **member != self.device)
            .filter(|member| self.peers.get(*member).is_none_or(|peer| peer.sessions.is_empty()))
            .cloned()
            .collect())
    }

    /// Starts a private session with `device`, using a prekey claimed from the server.
    pub fn start_session(&mut self, device: &str, prekey: &SignedKey) -> Result<(), Error> {
        let peer = self.peers.get_mut(device).ok_or_else(|| Error::UnknownPeer(device.to_owned()))?;
        let key = Curve25519PublicKey::from_base64(&prekey.key).map_err(|_| Error::BadKeys)?;
        let signature = Ed25519Signature::from_base64(&prekey.signature).map_err(|_| Error::BadKeys)?;
        peer.ed25519.verify(&prekey_message(device, &prekey.key), &signature).map_err(|_| Error::BadKeys)?;
        let session =
            self.account.create_outbound_session(olm_config(), peer.curve25519, key).map_err(|_| Error::BadKeys)?;
        remember(&mut peer.sessions, session, MAX_SESSIONS_PER_PEER);
        Ok(())
    }

    /// Asks `sender` to hand over their sending ratchet for `channel` again, for when a
    /// message of theirs arrived that this device has no keys for: the hand-over expired
    /// from the queue, or the private session it came over has gone out of step. Returns
    /// what to send to that one device.
    ///
    /// It goes over the newest private session with them, so start a new one first
    /// ([`Messenger::start_session`]) if the old one may be the problem. What was missed
    /// stays missed: the ratchet comes back at the position it has reached by then.
    pub fn ask_again(&mut self, channel: &str, sender: &str) -> Result<Vec<u8>, Error> {
        let roster = self.channels.get(channel).ok_or(Error::UnknownChannel)?.roster_version;
        let ask = Pairwise {
            from: self.device.clone(),
            to: sender.to_owned(),
            channel: channel.to_owned(),
            roster,
            key: String::new(),
            ask: true,
        };
        let peer = self.peers.get_mut(sender).ok_or_else(|| Error::UnknownPeer(sender.to_owned()))?;
        let session = peer.sessions.last_mut().ok_or_else(|| Error::NoSession(vec![sender.to_owned()]))?;
        let sealed = session
            .encrypt(serde_json::to_vec(&ask).expect("plain data"))
            .map_err(|_| Error::NoSession(vec![sender.to_owned()]))?;
        let (kind, bytes) = sealed.to_parts();
        let mut body = vec![PAIRWISE, kind as u8];
        body.extend(bytes);
        Ok(body)
    }

    /// Encrypts `plaintext` for everyone now in the channel.
    ///
    /// The first message after the roster changes (and every [`ROTATE_AFTER_MESSAGES`]
    /// messages or [`ROTATE_AFTER_SECONDS`]) starts a new sending ratchet and hands it
    /// to each member over their private session; those hand-overs are the `shares`.
    pub fn encrypt(&mut self, channel: &str, plaintext: &[u8], now: i64) -> Result<Outgoing, Error> {
        let padded = padding::pad(plaintext).ok_or(Error::TooLarge)?;
        let state = self.channels.get_mut(channel).ok_or(Error::UnknownChannel)?;
        if !state.members.contains(&self.device) {
            return Err(Error::NotAMember);
        }
        let recipients: Vec<String> = state.members.iter().filter(|member| **member != self.device).cloned().collect();
        let missing: Vec<String> = recipients
            .iter()
            .filter(|member| self.peers.get(*member).is_none_or(|peer| peer.sessions.is_empty()))
            .cloned()
            .collect();
        if !missing.is_empty() {
            return Err(Error::NoSession(missing));
        }

        let stale = state.outbound.as_ref().is_none_or(|outbound| {
            outbound.roster_version != state.roster_version
                || outbound.session.message_index() >= ROTATE_AFTER_MESSAGES
                || now - outbound.created >= ROTATE_AFTER_SECONDS
        });
        if stale {
            state.outbound = Some(Outbound {
                session: GroupSession::new(megolm_config()),
                roster_version: state.roster_version,
                created: now,
                shared_with: BTreeSet::new(),
            });
        }
        let outbound = state.outbound.as_mut().expect("just made if it was missing");

        let mut shares = Vec::new();
        for member in &recipients {
            if outbound.shared_with.contains(member) {
                continue;
            }
            let share = Pairwise {
                from: self.device.clone(),
                to: member.clone(),
                channel: channel.to_owned(),
                roster: outbound.roster_version,
                key: outbound.session.session_key().to_base64(),
                ask: false,
            };
            let peer = self.peers.get_mut(member).expect("checked above");
            let session = peer.sessions.last_mut().expect("checked above");
            let sealed = session
                .encrypt(serde_json::to_vec(&share).expect("plain data"))
                .map_err(|_| Error::NoSession(vec![member.clone()]))?;
            let (kind, bytes) = sealed.to_parts();
            let mut body = vec![PAIRWISE, kind as u8];
            body.extend(bytes);
            shares.push((member.clone(), body));
            outbound.shared_with.insert(member.clone());
        }

        let session_id = outbound.session.session_id();
        let sealed = outbound.session.encrypt(&padded).to_bytes();
        let mut message = vec![CHANNEL, session_id.len() as u8];
        message.extend(session_id.as_bytes());
        message.extend(sealed);
        Ok(Outgoing { shares, message })
    }

    /// Opens something that arrived in `channel`, said by the server to be from `sender`.
    ///
    /// Neither claim is trusted: a body only opens with keys that the real sender
    /// handed over for that channel. After a message is opened, the keys for it and for
    /// everything before it from that sender are destroyed.
    pub fn decrypt(&mut self, channel: &str, sender: &str, body: &[u8]) -> Result<Incoming, Error> {
        match body.split_first() {
            Some((&PAIRWISE, rest)) => self.receive_share(channel, sender, rest),
            Some((&CHANNEL, rest)) => self.receive_message(channel, sender, rest),
            _ => Err(Error::Malformed),
        }
    }

    fn receive_share(&mut self, channel: &str, sender: &str, body: &[u8]) -> Result<Incoming, Error> {
        let (&kind, bytes) = body.split_first().ok_or(Error::Malformed)?;
        let sealed = OlmMessage::from_parts(kind as usize, bytes).map_err(|_| Error::Malformed)?;
        let peer = self.peers.get_mut(sender).ok_or_else(|| Error::UnknownPeer(sender.to_owned()))?;

        let mut opened = None;
        for index in (0..peer.sessions.len()).rev() {
            let session = &mut peer.sessions[index];
            if let OlmMessage::PreKey(first) = &sealed
                && first.session_id() != session.session_id()
            {
                continue;
            }
            if let Ok(plaintext) = session.decrypt(&sealed) {
                let session = peer.sessions.remove(index);
                peer.sessions.push(session);
                opened = Some(plaintext);
                break;
            }
        }
        let plaintext = match (opened, &sealed) {
            (Some(plaintext), _) => plaintext,
            // The first message of a session the other device has just started.
            (None, OlmMessage::PreKey(first)) => {
                let created = self
                    .account
                    .create_inbound_session(olm_config(), peer.curve25519, first)
                    .map_err(|_| Error::CannotOpen)?;
                remember(&mut peer.sessions, created.session, MAX_SESSIONS_PER_PEER);
                created.plaintext
            }
            (None, OlmMessage::Normal(_)) => return Err(Error::CannotOpen),
        };

        let share: Pairwise = serde_json::from_slice(&plaintext).map_err(|_| Error::Malformed)?;
        if share.from != sender || share.to != self.device || share.channel != channel {
            return Err(Error::CannotOpen);
        }
        if share.ask {
            // Hand this device's ratchet to them again with the next message. Whether
            // they are (still) a member is decided there, as for every hand-over.
            if let Some(outbound) = self.channels.get_mut(channel).and_then(|state| state.outbound.as_mut()) {
                outbound.shared_with.remove(sender);
            }
            return Ok(Incoming::KeysWanted);
        }
        let key = SessionKey::from_base64(&share.key).map_err(|_| Error::Malformed)?;
        let session = InboundGroupSession::new(&key, megolm_config());
        // Kept whether or not the sender is in the roster held right now: this device
        // may simply not have fetched the newest roster yet. Membership is checked when
        // a message is opened.
        let state = self.channels.entry(channel.to_owned()).or_default();
        let held = state.inbound.entry(sender.to_owned()).or_default();
        held.retain(|existing| existing.session_id() != session.session_id());
        remember(held, session, MAX_INBOUND_PER_SENDER);
        Ok(Incoming::KeysReceived)
    }

    fn receive_message(&mut self, channel: &str, sender: &str, body: &[u8]) -> Result<Incoming, Error> {
        let (&id_len, rest) = body.split_first().ok_or(Error::Malformed)?;
        if rest.len() < id_len as usize {
            return Err(Error::Malformed);
        }
        let (session_id, sealed) = rest.split_at(id_len as usize);
        let sealed = MegolmMessage::from_bytes(sealed).map_err(|_| Error::Malformed)?;

        let state = self.channels.get_mut(channel).ok_or(Error::UnknownChannel)?;
        if !state.members.contains(sender) || !state.members.contains(&self.device) {
            return Err(Error::NotAMember);
        }
        let session = state
            .inbound
            .get_mut(sender)
            .and_then(|held| held.iter_mut().find(|session| session.session_id().as_bytes() == session_id))
            .ok_or(Error::UnknownSession)?;
        let opened = session.decrypt(&sealed).map_err(|error| match error {
            GroupDecryptionError::UnknownMessageIndex(..) => Error::AlreadyOpened,
            _ => Error::CannotOpen,
        })?;
        let plaintext = padding::unpad(&opened.plaintext).ok_or(Error::Malformed)?;
        // Forward secrecy: wind the ratchet past this message, so that neither it nor
        // anything earlier from this sender can be opened again, even with this
        // device's stored state.
        session.advance_to(opened.message_index.saturating_add(1));
        Ok(Incoming::Message(plaintext))
    }

    /// Everything this messenger holds, for the caller to store. **This is secret key
    /// material**: encrypt it before it touches a disk.
    pub fn to_bytes(&self) -> Vec<u8> {
        saved::save(self)
    }

    /// Restores a messenger from [`Messenger::to_bytes`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Messenger, Error> {
        saved::load(bytes).ok_or(Error::Malformed)
    }
}

/// Adds `item` as the newest of `held`, dropping the oldest beyond `limit`.
fn remember<T>(held: &mut Vec<T>, item: T, limit: usize) {
    held.push(item);
    if held.len() > limit {
        held.remove(0);
    }
}
