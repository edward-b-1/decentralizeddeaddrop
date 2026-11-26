//! Text encodings and random values.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

/// Standard base64 with padding: how keys, signatures and sealed bodies travel in JSON.
pub fn b64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

pub fn unb64(text: &str) -> Option<Vec<u8>> {
    STANDARD.decode(text).ok()
}

const BASE32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// Lowercase RFC 4648 base32 without padding: used for every ID and token, because it
/// survives being read aloud or typed.
pub fn base32(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 8 / 5 + 1);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            out.push(BASE32[((buffer >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(BASE32[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// Whether `text` could have come from [`base32`] and has exactly `len` characters.
pub fn is_base32(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| BASE32.contains(&b))
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("the operating system's random source failed");
    bytes
}

/// Length of [`random_id`].
pub const ID_LEN: usize = 26;

/// A new 128-bit identifier (channels, messages): 26 base32 characters.
pub fn random_id() -> String {
    base32(&random_bytes::<16>())
}

/// Length of [`random_token`].
pub const TOKEN_LEN: usize = 32;

/// A new 160-bit secret (invite tokens, session tokens): 32 base32 characters.
pub fn random_token() -> String {
    base32(&random_bytes::<20>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_matches_rfc4648_vectors() {
        // RFC 4648 section 10, lowercased and unpadded.
        assert_eq!(base32(b""), "");
        assert_eq!(base32(b"f"), "my");
        assert_eq!(base32(b"fo"), "mzxq");
        assert_eq!(base32(b"foo"), "mzxw6");
        assert_eq!(base32(b"foob"), "mzxw6yq");
        assert_eq!(base32(b"fooba"), "mzxw6ytb");
        assert_eq!(base32(b"foobar"), "mzxw6ytboi");
    }

    #[test]
    fn ids_have_the_advertised_shape() {
        let id = random_id();
        assert!(is_base32(&id, ID_LEN));
        assert!(is_base32(&random_token(), TOKEN_LEN));
        assert_ne!(id, random_id());
        assert!(!is_base32("K7F3", 4), "uppercase is not accepted");
    }
}
