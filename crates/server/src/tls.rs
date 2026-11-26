//! The server's TLS key and certificate (docs/PROTOCOL.md section 2).
//!
//! By default the server makes its own key and a self-signed certificate on first
//! start and keeps them beside the database. Phones recognise the server by the
//! fingerprint of that key, so no certificate authority or hostname is needed. An
//! operator who has a hostname and a certificate from an authority can supply those
//! files instead; the fingerprint works the same way.

use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use rustls::ServerConfig;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

pub struct Identity {
    pub config: Arc<ServerConfig>,
    /// What phones pin: see `deaddrop_tls::fingerprint`.
    pub fingerprint: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TlsError {
    /// A file is missing or unreadable.
    Read,
    /// The certificate file holds no certificate.
    NoCertificate,
    /// The key file holds no usable private key, or not the certificate's key.
    BadKey,
    /// The certificate is there but its key file is not. Without the key the server
    /// cannot prove it is the one phones know.
    KeyMissing,
    /// The new key or certificate could not be made or written.
    Create,
}

impl fmt::Display for TlsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            TlsError::Read => "cannot read the TLS certificate or key file",
            TlsError::NoCertificate => "the TLS certificate file holds no certificate",
            TlsError::BadKey => "the TLS key file does not hold the private key for the certificate",
            TlsError::KeyMissing => "the TLS key file is missing; restore it from your backup",
            TlsError::Create => "cannot create the TLS key and certificate files",
        })
    }
}

/// Loads a certificate (or chain) and its private key from PEM files.
pub fn load(certificate: &Path, key: &Path) -> Result<Identity, TlsError> {
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(certificate)
        .map_err(|_| TlsError::Read)?
        .collect::<Result<_, _>>()
        .map_err(|_| TlsError::NoCertificate)?;
    let first = chain.first().ok_or(TlsError::NoCertificate)?;
    let fingerprint = deaddrop_tls::fingerprint(first).ok_or(TlsError::NoCertificate)?;
    let key = PrivateKeyDer::from_pem_file(key).map_err(|error| match error {
        rustls::pki_types::pem::Error::Io(_) => TlsError::Read,
        _ => TlsError::BadKey,
    })?;

    let mut config = ServerConfig::builder_with_provider(deaddrop_tls::provider())
        // TLS 1.3 only: every client is ours, so there is nothing older to support.
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|_| TlsError::BadKey)?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(|_| TlsError::BadKey)?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Identity { config: Arc::new(config), fingerprint })
}

/// Loads the server's own key and certificate, making them first if neither exists.
/// Returns whether a new key was made just now.
///
/// The key is what matters. If only the certificate has gone missing, a new one is
/// made from the same key and the fingerprint does not change. If the key has gone
/// missing, nothing is made: a new key would be a different server as far as every
/// phone is concerned, so that has to be the operator's decision.
pub fn load_or_create(certificate: &Path, key: &Path) -> Result<(Identity, bool), TlsError> {
    let created = match (certificate.exists(), key.exists()) {
        (true, true) => false,
        (true, false) => return Err(TlsError::KeyMissing),
        (false, true) => {
            let pem = fs::read_to_string(key).map_err(|_| TlsError::Read)?;
            let key_pair = rcgen::KeyPair::from_pem(&pem).map_err(|_| TlsError::BadKey)?;
            write_certificate(certificate, &key_pair)?;
            false
        }
        (false, false) => {
            let key_pair = rcgen::KeyPair::generate().map_err(|_| TlsError::Create)?;
            write_private(key, key_pair.serialize_pem().as_bytes())?;
            write_certificate(certificate, &key_pair)?;
            true
        }
    };
    Ok((load(certificate, key)?, created))
}

/// Writes a self-signed certificate for `key_pair`. The name and dates in it are never
/// checked by anyone: phones go by the key's fingerprint.
fn write_certificate(certificate: &Path, key_pair: &rcgen::KeyPair) -> Result<(), TlsError> {
    let made = rcgen::CertificateParams::new(vec!["deaddrop".to_owned()])
        .and_then(|params| params.self_signed(key_pair))
        .map_err(|_| TlsError::Create)?;
    fs::write(certificate, made.pem()).map_err(|_| TlsError::Create)
}

/// Writes a file only its owner can read (on systems that have such a thing), and
/// never over an existing file.
fn write_private(path: &Path, contents: &[u8]) -> Result<(), TlsError> {
    use std::io::Write;
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(|_| TlsError::Create)?;
    file.write_all(contents).map_err(|_| TlsError::Create)
}
