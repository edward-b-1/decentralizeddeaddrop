//! Device public keys and signatures.
//!
//! Devices hold ECDSA P-256 keys (on Android, in the Keystore). Public keys travel as
//! DER SubjectPublicKeyInfo and signatures as ASN.1 DER ECDSA over SHA-256, which is
//! exactly what Android's `SHA256withECDSA` produces.

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

#[cfg(test)]
mod tests {
    use super::*;
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
