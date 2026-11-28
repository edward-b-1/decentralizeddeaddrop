//! The Dead Drop client engine: everything an app does apart from drawing screens and
//! holding the device's hardware keys.
//!
//! An [`Account`] is one identity on one server. It registers, signs in, keeps channels
//! and their member lists up to date, and sends and receives messages. Every claim a
//! server makes is checked here before it is believed (docs/PROTOCOL.md): the server's
//! key against the fingerprint, the channel's owner against the invite, the roster
//! against the owner's signature, and each member's keys against the roster.
//!
//! The engine is plain blocking code: call it from a background thread.

mod account;
mod http;
pub mod link;
pub mod live;

use deaddrop_core::encoding::b64;
use deaddrop_core::soft::SoftDevice;

pub use account::{Account, Channel, Event, JoinRequest, NotSent, Save, ServerDevice, WayIn, WaysIn, Why};
pub use link::{ChannelInvite, Link, ServerAddress};
pub use live::{Live, Wake};

/// The device's two long-term keys. On a phone these live in the Android Keystore and
/// this trait is implemented in Kotlin; the private halves never reach this code.
pub trait DeviceKeys {
    /// The signing public key, DER SubjectPublicKeyInfo (P-256).
    fn signing_key_der(&self) -> Vec<u8>;
    /// The agreement public key, DER SubjectPublicKeyInfo (P-256).
    fn agreement_key_der(&self) -> Vec<u8>;
    /// The signing key's signature over `keys::agreement_key_message`, base64.
    fn agreement_signature(&self) -> String;
    /// A DER ECDSA signature over SHA-256(`message`), made with the signing key.
    fn sign(&self, message: &[u8]) -> Vec<u8>;
}

impl DeviceKeys for SoftDevice {
    fn signing_key_der(&self) -> Vec<u8> {
        SoftDevice::signing_key_der(self)
    }
    fn agreement_key_der(&self) -> Vec<u8> {
        SoftDevice::agreement_key_der(self)
    }
    fn agreement_signature(&self) -> String {
        SoftDevice::agreement_signature(self)
    }
    fn sign(&self, message: &[u8]) -> Vec<u8> {
        SoftDevice::sign(self, message)
    }
}

pub(crate) fn key_b64(der: &[u8]) -> String {
    b64(der)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    /// Not a usable `deaddrop://` link or server address.
    BadLink,
    /// The server could not be reached, or what answered is not the server with the
    /// expected fingerprint. Deliberately one error: either way, nothing was sent.
    Unreachable,
    /// The server said no. `message` is its own short explanation.
    Refused {
        status: u16,
        message: String,
    },
    /// The server has registered this device but its operator has not approved it yet.
    AwaitingApproval,
    /// The server presented something that fails a check: a wrong owner, a bad
    /// signature, an older record than one already seen, keys that do not belong.
    Untrusted(&'static str),
    /// A member has not published the keys needed to message them yet.
    MemberNotReady(String),
    /// The channel is not one this account is in.
    UnknownChannel,
    /// Only the channel's owner can do that.
    NotOwner,
    /// The link is for a different server from this account's.
    WrongServer,
    Crypto(deaddrop_crypto::Error),
    /// Saved state that this version cannot read.
    BadState,
    /// What was asked for cannot be done; the text says why, for the person asking.
    Invalid(&'static str),
}

impl From<deaddrop_crypto::Error> for ClientError {
    fn from(error: deaddrop_crypto::Error) -> ClientError {
        ClientError::Crypto(error)
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::BadLink => f.write_str("that is not a complete deaddrop:// link"),
            ClientError::Unreachable => {
                f.write_str("cannot reach the server, or what answered is not the server with that fingerprint")
            }
            ClientError::Refused { status, message } => write!(f, "the server refused ({status}): {message}"),
            ClientError::AwaitingApproval => f.write_str("waiting for the server's operator to approve this device"),
            ClientError::Untrusted(what) => write!(f, "the server sent something that does not check out: {what}"),
            ClientError::MemberNotReady(device) => write!(f, "member {device} has not published their keys yet"),
            ClientError::UnknownChannel => f.write_str("no such channel"),
            ClientError::NotOwner => f.write_str("only the channel's owner can do that"),
            ClientError::WrongServer => f.write_str("that link is for a different server"),
            ClientError::Crypto(error) => write!(f, "encryption: {error:?}"),
            ClientError::BadState => f.write_str("the saved state cannot be read"),
            ClientError::Invalid(why) => f.write_str(why),
        }
    }
}
