//! The optional server password. Only an Argon2id hash is kept.

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};

/// Hashes a password to a PHC string with a random salt. Slow by design.
pub fn hash(password: &str) -> Option<String> {
    Argon2::default().hash_password(password.as_bytes()).ok().map(|hash| hash.to_string())
}

/// Slow by design; run it off the async threads.
pub fn verify(password: &str, stored: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(stored) else { return false };
    Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_right_password_verifies() {
        let stored = hash("open sesame").unwrap();
        assert!(stored.starts_with("$argon2id$"));
        assert!(verify("open sesame", &stored));
        assert!(!verify("open sesame!", &stored));
        assert!(!verify("open sesame", "not a hash"));
    }
}
