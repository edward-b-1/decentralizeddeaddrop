//! Records signed by a channel's owner.
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
