//! Server fingerprints, and the check a phone makes with one (docs/PROTOCOL.md section 2).
//!
//! A server's **fingerprint** is the SHA-256 of its TLS public key, written in base32
//! (52 characters). It is the hash of the key, not of the certificate, so it stays the
//! same when a certificate is renewed and changes only if the server's key does.
//!
//! A phone that has a server's fingerprint (from an invite link or QR code) does not
//! need a certificate authority, a hostname, or the server's clock to be right: it
//! accepts exactly the server that holds that key and no other. That is what
//! [`pinned_client_config`] builds.

use std::sync::Arc;

use deaddrop_core::encoding::{base32, is_base32};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::server::ParsedCertificate;
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, Error, SignatureScheme};
use sha2::{Digest, Sha256};

/// Characters in a fingerprint.
pub const FINGERPRINT_LEN: usize = 52;

/// The cryptography every Dead Drop TLS connection uses.
pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// The fingerprint of the key in a certificate; `None` if it is not a certificate.
pub fn fingerprint(certificate: &CertificateDer<'_>) -> Option<String> {
    let parsed = ParsedCertificate::try_from(certificate).ok()?;
    Some(base32(&Sha256::digest(parsed.subject_public_key_info())))
}

/// A fingerprint in groups of four for people to read and compare:
/// `abcd-efgh-...`.
pub fn display(fingerprint: &str) -> String {
    let characters: Vec<char> = fingerprint.chars().collect();
    characters.chunks(4).map(|group| group.iter().collect::<String>()).collect::<Vec<_>>().join("-")
}

/// Accepts a fingerprint as typed or pasted, with any grouping, spaces or capitals.
pub fn parse(typed: &str) -> Option<String> {
    let plain: String =
        typed.chars().filter(|c| !c.is_whitespace() && *c != '-').map(|c| c.to_ascii_lowercase()).collect();
    is_base32(&plain, FINGERPRINT_LEN).then_some(plain)
}

/// Accepts a server if, and only if, its key has the expected fingerprint.
#[derive(Debug)]
struct Pinned {
    fingerprint: String,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        // Deliberately not checked: who issued the certificate, the name in it, and its
        // dates. The fingerprint identifies the server completely. The server still has
        // to prove it holds the private key, which `verify_tls13_signature` checks.
        match fingerprint(end_entity) {
            Some(found) if found == self.fingerprint => Ok(ServerCertVerified::assertion()),
            _ => Err(Error::InvalidCertificate(CertificateError::ApplicationVerificationFailure)),
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _certificate: &CertificateDer<'_>,
        _signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        // Only TLS 1.3 is offered, so this is never reached.
        Err(Error::General("TLS 1.2 is not used".into()))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(message, certificate, signature, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// A TLS 1.3 client configuration that connects only to the server whose key has this
/// fingerprint. `None` if `fingerprint` is not a fingerprint.
pub fn pinned_client_config(fingerprint: &str) -> Option<ClientConfig> {
    let fingerprint = parse(fingerprint)?;
    let provider = provider();
    let algorithms = provider.signature_verification_algorithms;
    let config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .ok()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned { fingerprint, algorithms }))
        .with_no_client_auth();
    Some(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_can_be_read_out_and_typed_back_in() {
        let fingerprint = "abcdefghijklmnopqrstuvwxyz234567abcdefghijklmnopqrst";
        assert_eq!(fingerprint.len(), FINGERPRINT_LEN);
        let shown = display(fingerprint);
        assert_eq!(shown, "abcd-efgh-ijkl-mnop-qrst-uvwx-yz23-4567-abcd-efgh-ijkl-mnop-qrst");
        assert_eq!(parse(&shown).as_deref(), Some(fingerprint));
        assert_eq!(parse(&shown.to_uppercase().replace('-', " ")).as_deref(), Some(fingerprint));
        assert_eq!(parse("too-short"), None);
        assert_eq!(parse(&fingerprint.replace('a', "1")), None, "1 is not a base32 character");
    }

    #[test]
    fn something_that_is_not_a_certificate_has_no_fingerprint() {
        assert_eq!(fingerprint(&CertificateDer::from(b"not a certificate".to_vec())), None);
        assert!(pinned_client_config("not a fingerprint").is_none());
    }
}
