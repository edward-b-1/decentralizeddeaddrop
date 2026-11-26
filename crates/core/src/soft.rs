//! A device whose keys live in ordinary memory, for tests and command-line clients.
//! Phones keep theirs in the Android Keystore instead.

use p256::ecdsa::signature::Signer;
use p256::ecdsa::{Signature, SigningKey};
use p256::pkcs8::EncodePublicKey;

use crate::encoding::{b64, random_bytes};
use crate::keys;
use crate::records::{Record, Signed};

pub struct SoftDevice {
    signing: SigningKey,
    agreement: SigningKey,
}

fn new_key() -> SigningKey {
    loop {
        // A random 32-byte string is a valid P-256 private key except with negligible odds.
        if let Ok(key) = SigningKey::from_slice(&random_bytes::<32>()) {
            return key;
        }
    }
}

fn public_der(key: &SigningKey) -> Vec<u8> {
    key.verifying_key().to_public_key_der().expect("P-256 keys always encode").as_bytes().to_vec()
}

impl Default for SoftDevice {
    fn default() -> Self {
        Self::new()
    }
}

impl SoftDevice {
    pub fn new() -> SoftDevice {
        SoftDevice { signing: new_key(), agreement: new_key() }
    }

    /// Both private keys, for a command-line client to keep in its state file.
    /// A phone never does this: its keys cannot leave the secure hardware.
    pub fn to_bytes(&self) -> Vec<u8> {
        [self.signing.to_bytes(), self.agreement.to_bytes()].concat()
    }

    /// Restores a device from [`SoftDevice::to_bytes`].
    pub fn from_bytes(bytes: &[u8]) -> Option<SoftDevice> {
        let (signing, agreement) = bytes.split_at_checked(32)?;
        Some(SoftDevice {
            signing: SigningKey::from_slice(signing).ok()?,
            agreement: SigningKey::from_slice(agreement).ok()?,
        })
    }

    pub fn signing_key_der(&self) -> Vec<u8> {
        public_der(&self.signing)
    }

    /// The agreement key is a second P-256 key; only its public half matters here.
    pub fn agreement_key_der(&self) -> Vec<u8> {
        public_der(&self.agreement)
    }

    pub fn device_id(&self) -> String {
        keys::device_id(&self.signing_key_der())
    }

    /// DER ECDSA signature over SHA-256(`message`).
    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        let signature: Signature = self.signing.sign(message);
        signature.to_der().as_bytes().to_vec()
    }

    /// The signing key's voucher for the agreement key, base64.
    pub fn agreement_signature(&self) -> String {
        b64(&self.sign(&keys::agreement_key_message(&self.agreement_key_der())))
    }

    pub fn seal<R: Record>(&self, record: &R) -> Signed {
        Signed::seal(record, |message| self.sign(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::{ChannelRecord, RecordError, Roster, RosterMember};

    #[test]
    fn signatures_verify_only_for_the_right_key_and_message() {
        let (a, b) = (SoftDevice::new(), SoftDevice::new());
        let signature = a.sign(b"hello");
        assert!(keys::verify(&a.signing_key_der(), b"hello", &signature));
        assert!(!keys::verify(&a.signing_key_der(), b"hellp", &signature));
        assert!(!keys::verify(&b.signing_key_der(), b"hello", &signature));
        assert!(!keys::verify(b"not a key", b"hello", &signature));
        assert!(!keys::verify(&a.signing_key_der(), b"hello", b"not a signature"));
    }

    #[test]
    fn a_saved_device_is_the_same_device() {
        let device = SoftDevice::new();
        let restored = SoftDevice::from_bytes(&device.to_bytes()).unwrap();
        assert_eq!(restored.device_id(), device.device_id());
        assert_eq!(restored.agreement_key_der(), device.agreement_key_der());
        assert!(keys::verify(&device.signing_key_der(), b"hello", &restored.sign(b"hello")));
        assert!(SoftDevice::from_bytes(b"too short").is_none());
    }

    #[test]
    fn device_id_is_sixteen_base32_characters() {
        let device = SoftDevice::new();
        assert!(crate::encoding::is_base32(&device.device_id(), keys::DEVICE_ID_LEN));
    }

    #[test]
    fn records_open_only_with_the_signers_key() {
        let (owner, other) = (SoftDevice::new(), SoftDevice::new());
        let record = ChannelRecord {
            v: 1,
            id: crate::encoding::random_id(),
            name: "Family".into(),
            owner: owner.device_id(),
            max_age: 86_400,
            version: 1,
            issued: 1_800_000_000,
        };
        let signed = owner.seal(&record);
        assert_eq!(signed.open::<ChannelRecord>(&owner.signing_key_der()), Ok(record));
        assert_eq!(signed.open::<ChannelRecord>(&other.signing_key_der()), Err(RecordError::BadSignature));

        let mut altered = signed.clone();
        altered.record = altered.record.replace("86400", "99999");
        assert_eq!(altered.open::<ChannelRecord>(&owner.signing_key_der()), Err(RecordError::BadSignature));
    }

    #[test]
    fn a_signature_on_one_kind_of_record_does_not_open_as_another() {
        let owner = SoftDevice::new();
        let roster = Roster {
            v: 1,
            channel: crate::encoding::random_id(),
            version: 1,
            members: vec![RosterMember { device: owner.device_id(), name: "Owner".into() }],
            issued: 1_800_000_000,
        };
        let signed = owner.seal(&roster);
        assert!(signed.open::<Roster>(&owner.signing_key_der()).is_ok());
        assert_eq!(signed.open::<ChannelRecord>(&owner.signing_key_der()), Err(RecordError::BadSignature));
    }

    #[test]
    fn unknown_record_versions_are_refused() {
        let owner = SoftDevice::new();
        let roster = Roster { v: 2, channel: "x".into(), version: 1, members: vec![], issued: 0 };
        assert_eq!(owner.seal(&roster).open::<Roster>(&owner.signing_key_der()), Err(RecordError::UnknownVersion));
    }
}
