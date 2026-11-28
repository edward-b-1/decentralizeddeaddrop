//! Signed records: a channel's settings and member list, signed by its owner, and what
//! a device signs for itself (its messaging keys, and its word that it is leaving).
//!
//! A record travels as the exact JSON text that was signed, plus the signature
//! ([`Signed`]). Readers verify the signature over that text and only then parse it, so
//! there is no canonical form to disagree about.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::encoding::{b64, unb64};
use crate::keys;

/// A record as signed text plus its base64 DER signature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signed {
    pub record: String,
    pub signature: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RecordError {
    BadSignature,
    Malformed,
    UnknownVersion,
}

/// A kind of signed record. `CONTEXT` is prefixed to the text before signing, so a
/// signature on one kind of record can never be passed off as another kind.
pub trait Record: Serialize + DeserializeOwned {
    const CONTEXT: &'static str;
    fn format_version(&self) -> u32;
}

/// The bytes that are actually signed for a record of type `R`.
pub fn signing_message<R: Record>(record_text: &str) -> Vec<u8> {
    format!("{}\n{record_text}", R::CONTEXT).into_bytes()
}

impl Signed {
    /// Serialises `record` and signs it with `sign` (which returns a DER signature).
    pub fn seal<R: Record>(record: &R, sign: impl FnOnce(&[u8]) -> Vec<u8>) -> Signed {
        let text = serde_json::to_string(record).expect("records are plain data");
        let signature = b64(&sign(&signing_message::<R>(&text)));
        Signed { record: text, signature }
    }

    /// Verifies the signature against `signer_key_der`, then parses the record.
    pub fn open<R: Record>(&self, signer_key_der: &[u8]) -> Result<R, RecordError> {
        let signature = unb64(&self.signature).ok_or(RecordError::BadSignature)?;
        if !keys::verify(signer_key_der, &signing_message::<R>(&self.record), &signature) {
            return Err(RecordError::BadSignature);
        }
        let record: R = serde_json::from_str(&self.record).map_err(|_| RecordError::Malformed)?;
        if record.format_version() != 1 {
            return Err(RecordError::UnknownVersion);
        }
        Ok(record)
    }
}

/// A channel's settings, signed by its owner.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelRecord {
    pub v: u32,
    /// 26 base32 characters, chosen at random by the owner's device.
    pub id: String,
    pub name: String,
    /// Device ID of the owner. Because it is a hash of the owner's signing key, an invite
    /// that carries the channel ID and this value pins who may sign for the channel.
    pub owner: String,
    /// Maximum message age in seconds: how long an undelivered message stays queued.
    pub max_age: u64,
    /// Goes up with every change; readers never accept a lower one.
    pub version: u64,
    /// Unix seconds, as set by the owner's device.
    pub issued: i64,
}

impl Record for ChannelRecord {
    const CONTEXT: &'static str = "deaddrop/channel/v1";
    fn format_version(&self) -> u32 {
        self.v
    }
}

/// One member in a [`Roster`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterMember {
    pub device: String,
    /// The name this member goes by in this channel.
    pub name: String,
    /// The version of the roster that let this member in. A [`Leave`] they signed
    /// before that belongs to an earlier time in the channel and no longer counts.
    /// Absent (0) in rosters signed by versions up to 0.1.1.
    #[serde(default)]
    pub since: u64,
}

/// The complete member list of a channel, signed by its owner. Every membership change
/// is a new roster with a higher version. Because readers never accept a lower version,
/// a server can neither add a member nor keep a removed one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Roster {
    pub v: u32,
    pub channel: String,
    pub version: u64,
    pub members: Vec<RosterMember>,
    pub issued: i64,
}

impl Record for Roster {
    const CONTEXT: &'static str = "deaddrop/roster/v1";
    fn format_version(&self) -> u32 {
        self.v
    }
}

/// A member's own word that they are leaving a channel, signed with their device's
/// signing key (the one its device ID is the hash of).
///
/// Only the owner signs the member list, and the owner's phone drops a member from it
/// without being asked only on the strength of one of these. A server that merely says
/// someone has left gets nothing signed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Leave {
    pub v: u32,
    pub channel: String,
    /// Device ID of the member leaving.
    pub device: String,
    /// The version of the roster the device held when it left.
    pub roster: u64,
    pub issued: i64,
}

impl Record for Leave {
    const CONTEXT: &'static str = "deaddrop/leave/v1";
    fn format_version(&self) -> u32 {
        self.v
    }
}

/// Checks a signed [`Leave`] the way a phone must before believing it: the signing key
/// really is the one the member's device ID is named after, the record is signed by it,
/// it is about this channel and this member, and it was signed since the member was
/// last let in (so a leave from an earlier time in the channel cannot be played again).
pub fn open_leave(
    channel: &str,
    member: &RosterMember,
    signing_key_der: &[u8],
    signed: &Signed,
) -> Result<Leave, RecordError> {
    if keys::device_id(signing_key_der) != member.device {
        return Err(RecordError::BadSignature);
    }
    let leave: Leave = signed.open(signing_key_der)?;
    if leave.channel != channel || leave.device != member.device || leave.roster < member.since {
        return Err(RecordError::BadSignature);
    }
    Ok(leave)
}

/// A device's messaging keys, signed by its signing key (the one its device ID is the
/// hash of). The signing and agreement keys of `keys.rs` never leave the phone's secure
/// hardware; these two are what the message encryption works with day to day, and this
/// record is what ties them to the device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessagingKeys {
    pub v: u32,
    pub device: String,
    /// Curve25519 identity key, unpadded base64.
    pub curve25519: String,
    /// Ed25519 key that signs this device's prekeys, unpadded base64.
    pub ed25519: String,
    pub issued: i64,
}

impl Record for MessagingKeys {
    const CONTEXT: &'static str = "deaddrop/messaging-keys/v1";
    fn format_version(&self) -> u32 {
        self.v
    }
}

/// A prekey: a Curve25519 key a device leaves on the server so that others can start a
/// private session with it while it is offline. Signed with the Ed25519 key in the
/// device's [`MessagingKeys`], so a server cannot swap in a key of its own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedKey {
    /// Curve25519 public key, unpadded base64.
    pub key: String,
    /// Ed25519 signature over [`prekey_message`], unpadded base64.
    pub signature: String,
}

/// What a device signs for each of its prekeys.
pub fn prekey_message(device: &str, key: &str) -> Vec<u8> {
    format!("deaddrop/prekey/v1\n{device}\n{key}").into_bytes()
}

/// Checks a device's signed messaging keys the way every phone must before trusting
/// them: the signing key really is the one `device` is named after, the record is
/// signed by it, and the record is about that device.
///
/// With `device` taken from an owner-signed roster, this is the chain that ties the
/// keys messages are encrypted to back to the channel owner's signature.
pub fn open_messaging_keys(
    device: &str,
    signing_key_der: &[u8],
    signed: &Signed,
) -> Result<MessagingKeys, RecordError> {
    if keys::device_id(signing_key_der) != device {
        return Err(RecordError::BadSignature);
    }
    let record: MessagingKeys = signed.open(signing_key_der)?;
    if record.device != device {
        return Err(RecordError::BadSignature);
    }
    Ok(record)
}
