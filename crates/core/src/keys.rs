//! Device public keys and signatures.
//!
//! Devices hold ECDSA P-256 keys (on Android, in the Keystore). Public keys travel as
//! DER SubjectPublicKeyInfo and signatures as ASN.1 DER ECDSA over SHA-256, which is
//! exactly what Android's `SHA256withECDSA` produces.

use hmac::{Hmac, KeyInit, Mac};
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use p256::pkcs8::DecodePublicKey;
use sha2::{Digest, Sha256};

use crate::encoding::{b64, base32};

/// Length of a device ID.
pub const DEVICE_ID_LEN: usize = 16;

/// Parses a DER SubjectPublicKeyInfo P-256 key; `None` for anything else.
pub fn parse_public_key(der: &[u8]) -> Option<VerifyingKey> {
    VerifyingKey::from_public_key_der(der).ok()
}

/// A short, stable name for a device, derived from its signing key: base32 of the first
/// ten bytes of SHA-256(DER). Because the ID is a hash of the key, naming a device in a
/// signed record also pins its key.
pub fn device_id(signing_key_der: &[u8]) -> String {
    base32(&Sha256::digest(signing_key_der)[..10])
}

/// Checks an ASN.1 DER ECDSA signature over SHA-256(`message`).
pub fn verify(signing_key_der: &[u8], message: &[u8], signature_der: &[u8]) -> bool {
    let Some(key) = parse_public_key(signing_key_der) else { return false };
    let Ok(signature) = Signature::from_der(signature_der) else { return false };
    key.verify(message, &signature).is_ok()
}

/// What a device's signing key signs to vouch for its agreement (ECDH) key, so a server
/// cannot swap in an agreement key of its own.
pub fn agreement_key_message(agreement_key_der: &[u8]) -> Vec<u8> {
    format!("deaddrop/agreement-key/v1\n{}", b64(agreement_key_der)).into_bytes()
}

/// What a device signs to open a session. Naming the server means a signature made for
/// one server is useless at another.
pub fn session_message(server_id: &str, challenge: &str) -> Vec<u8> {
    format!("deaddrop/session/v1\n{server_id}\n{challenge}").into_bytes()
}

fn join_mac(secret: &str, channel: &str, device: &str, name: &str) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC takes a key of any length");
    // The name comes last because it is the one part that is free text.
    mac.update(format!("deaddrop/join/v1\n{channel}\n{device}\n{name}").as_bytes());
    mac
}

/// The stamp a device puts on its request to join a channel: HMAC-SHA256, keyed with
/// the secret in the invite it holds, over the channel, its own device ID and the name
/// it asks for. Base64.
///
/// The secret travels inside the invite, from the owner's phone to the joiner's, and is
/// never sent to the server. So a stamp that checks out tells the owner's phone that
/// this device, under this name, is asking with an invite the owner made; a server can
/// neither make one nor move one to another device or name.
pub fn join_stamp(secret: &str, channel: &str, device: &str, name: &str) -> String {
    b64(&join_mac(secret, channel, device, name).finalize().into_bytes())
}

/// Whether `stamp` is the [`join_stamp`] for exactly this secret, channel, device and name.
pub fn join_stamp_matches(secret: &str, channel: &str, device: &str, name: &str, stamp: &str) -> bool {
    let Some(stamp) = crate::encoding::unb64(stamp) else { return false };
    join_mac(secret, channel, device, name).verify_slice(&stamp).is_ok()
}

/// The safety number two devices share: thirty digits in six groups, the same on both
/// phones whichever way round they are asked.
///
/// A device ID is a hash of that device's signing key, and every other key a device
/// uses is signed with it. So if two people read the same number on their two phones,
/// each phone holds the other's real keys, and no server or third person has put a
/// device of their own in between.
pub fn safety_number(one: &str, other: &str) -> String {
    let (first, second) = if one <= other { (one, other) } else { (other, one) };
    let digest = Sha256::digest(format!("deaddrop/safety/v1\n{first}\n{second}"));
    let groups = digest.chunks(5).take(6).map(|chunk| {
        let number = chunk.iter().fold(0u64, |number, byte| (number << 8) | u64::from(*byte));
        format!("{:05}", number % 100_000)
    });
    groups.collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_join_stamp_fits_one_secret_channel_device_and_name() {
        let stamp = join_stamp("secret", "channel", "device", "Ben");
        assert!(join_stamp_matches("secret", "channel", "device", "Ben", &stamp));
        assert!(!join_stamp_matches("other", "channel", "device", "Ben", &stamp));
        assert!(!join_stamp_matches("secret", "elsewhere", "device", "Ben", &stamp));
        assert!(!join_stamp_matches("secret", "channel", "another", "Ben", &stamp));
        assert!(!join_stamp_matches("secret", "channel", "device", "Bob", &stamp));
        assert!(!join_stamp_matches("secret", "channel", "device", "Ben", ""));
        assert!(!join_stamp_matches("secret", "channel", "device", "Ben", "not base64!"));
        // RFC 4231 test case 2 pins the construction: HMAC-SHA256, nothing home-made.
        let mut mac = Hmac::<Sha256>::new_from_slice(b"Jefe").unwrap();
        mac.update(b"what do ya want for nothing?");
        let hex: String = mac.finalize().into_bytes().iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(hex, "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    #[test]
    fn a_safety_number_is_the_same_from_both_ends_and_differs_for_anyone_else() {
        let (anna, ben, eve) = ("t2n6vrwrnowcmdaw", "abcdefghijklmnop", "abcdefghijklmnoq");
        let number = safety_number(anna, ben);
        assert_eq!(number, safety_number(ben, anna));
        assert_eq!(number.len(), 35);
        assert!(number.split(' ').all(|group| group.len() == 5 && group.chars().all(|c| c.is_ascii_digit())));
        assert_ne!(number, safety_number(anna, eve));
        // Fixed, so that a later version cannot change it by accident.
        assert_eq!(number, "43792 05729 00507 13378 10572 01825");
    }
    use crate::encoding::unb64;

    // A key and two signatures made by the JDK's `SHA256withECDSA`, the same provider
    // interface Android uses. Java does not normalise signatures, so one has a "high" S
    // value; both must verify or some phones' signatures would be refused at random.
    const JAVA_KEY: &str = "MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAELl5DYHboHTvIIMN8KHts9WbGGheHzRQdG5ml1ijiOcdS7NM2h1VlTqiMMxH4lXBDhiWYVZoCFHnMkTNRM9eo+w==";
    const JAVA_MESSAGE: &[u8] = b"deaddrop/test-vector/v1\nsigned by the JDK, as Android signs";
    const JAVA_LOW_S: &str =
        "MEUCIQDWKZ7PgPsH6Frk/A/imDAADVHfl2XCCt/ncd2nEATUSgIgBlO9bGm5amsVjogm2RAtrWr8RmW22P+5vhxewLW6oiI=";
    const JAVA_HIGH_S: &str =
        "MEUCICqQD0vH75+PnqSWcT61ZNWp0b8rrN0wgzOaxnTnfb+dAiEAi23df7/aQwOvK19Ob8stebWXjXoU8OcCGAiIHcER5Eg=";

    #[test]
    fn signatures_made_by_java_verify() {
        let key = unb64(JAVA_KEY).unwrap();
        assert!(parse_public_key(&key).is_some());
        assert!(verify(&key, JAVA_MESSAGE, &unb64(JAVA_LOW_S).unwrap()), "low S");
        assert!(verify(&key, JAVA_MESSAGE, &unb64(JAVA_HIGH_S).unwrap()), "high S");
        assert!(!verify(&key, b"another message", &unb64(JAVA_LOW_S).unwrap()));
    }

    #[test]
    fn device_id_matches_the_documented_derivation() {
        // base32(SHA-256(key)[0..10]), lowercase; expected value worked out with Python's
        // hashlib and base64 modules.
        let id = device_id(&unb64(JAVA_KEY).unwrap());
        assert_eq!(id.len(), DEVICE_ID_LEN);
        assert_eq!(id, "t2n6vrwrnowcmdaw");
    }
}
