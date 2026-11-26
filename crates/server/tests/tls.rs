//! TLS and the server fingerprint: a phone that holds a fingerprint talks to exactly
//! the server with that key, and to nobody who has taken over its address.

mod common;

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use common::*;
use deaddrop_core::soft::SoftDevice;
use deaddropd::tls::{self, TlsError};
use futures_util::{SinkExt, StreamExt};
use rustls::pki_types::ServerName;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_tungstenite::tungstenite::Message;

struct Running {
    server: Server,
    address: SocketAddr,
    fingerprint: String,
}

/// A server with its key and certificate in `directory`, listening on a real port.
async fn start(directory: &Path) -> Running {
    let server = Server::new().await;
    let (identity, _) = tls::load_or_create(&directory.join("cert.pem"), &directory.join("key.pem")).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(deaddropd::serve_tls(listener, server.state.clone(), identity.config.clone(), std::future::pending()));
    Running { server, address, fingerprint: identity.fingerprint }
}

/// Connects the way a phone does: it knows an address and a fingerprint, nothing else.
async fn connect(address: SocketAddr, fingerprint: &str) -> std::io::Result<TlsStream<TcpStream>> {
    let config = deaddrop_tls::pinned_client_config(fingerprint).expect("a well-formed fingerprint");
    let stream = TcpStream::connect(address).await?;
    TlsConnector::from(Arc::new(config)).connect(ServerName::from(address.ip()), stream).await
}

/// One HTTP request over an established connection; returns the status and JSON body.
async fn request(mut stream: TlsStream<TcpStream>, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
    let body = body.map(|body| body.to_string()).unwrap_or_default();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: deaddrop\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    // The server may close without a TLS goodbye once it has answered; what was read counts.
    let _ = stream.read_to_end(&mut response).await;
    let response = String::from_utf8(response).unwrap();
    let status = response.split(' ').nth(1).expect("an HTTP status line").parse().unwrap();
    let json = response.split_once("\r\n\r\n").map(|(_, body)| body).unwrap_or_default();
    (status, serde_json::from_str(json).unwrap_or(Value::Null))
}

#[tokio::test]
async fn a_phone_with_the_fingerprint_reaches_the_server() {
    let scratch = Scratch::new();
    let running = start(&scratch.0).await;
    let stream = connect(running.address, &running.fingerprint).await.unwrap();
    let (status, info) = request(stream, "GET", "/v1/server", None).await;
    assert_eq!(status, 200);
    assert_eq!(info["server_id"], json!(&*running.server.state.server_id));

    // The fingerprint also works as people would type it from a screen.
    let typed = deaddrop_tls::display(&running.fingerprint).to_uppercase();
    assert!(connect(running.address, &typed).await.is_ok());
}

#[tokio::test]
async fn a_phone_with_a_different_fingerprint_refuses_to_connect() {
    let scratch = Scratch::new();
    let running = start(&scratch.0).await;
    let mut wrong: Vec<char> = running.fingerprint.chars().collect();
    wrong[0] = if wrong[0] == 'a' { 'b' } else { 'a' };
    let wrong: String = wrong.into_iter().collect();
    let refused = connect(running.address, &wrong).await;
    assert!(refused.is_err(), "one changed character is enough");
}

#[tokio::test]
async fn someone_who_takes_over_the_address_is_not_the_server() {
    // Two servers stand in for the real one and an impostor now answering at its
    // address. The impostor has everything except the real server's private key.
    let (real_files, impostor_files) = (Scratch::new(), Scratch::new());
    let real = start(&real_files.0).await;
    let impostor = start(&impostor_files.0).await;
    assert_ne!(real.fingerprint, impostor.fingerprint);

    assert!(connect(impostor.address, &real.fingerprint).await.is_err(), "the phone will not talk to the impostor");
    assert!(connect(real.address, &real.fingerprint).await.is_ok());

    // Even an impostor who copies the real server's certificate gets nowhere: the
    // certificate is public, and without the key behind it the handshake cannot finish.
    let copy = Scratch::new();
    std::fs::copy(real_files.0.join("cert.pem"), copy.0.join("cert.pem")).unwrap();
    std::fs::copy(impostor_files.0.join("key.pem"), copy.0.join("key.pem")).unwrap();
    assert_eq!(tls::load(&copy.0.join("cert.pem"), &copy.0.join("key.pem")).err(), Some(TlsError::BadKey));
}

#[tokio::test]
async fn the_fingerprint_belongs_to_the_key_and_outlives_the_certificate() {
    let scratch = Scratch::new();
    let (certificate, key) = (scratch.0.join("cert.pem"), scratch.0.join("key.pem"));
    let (first, created) = tls::load_or_create(&certificate, &key).unwrap();
    assert!(created);
    assert_eq!(first.fingerprint.len(), deaddrop_tls::FINGERPRINT_LEN);

    // A restart finds the same files and is the same server.
    let (again, created) = tls::load_or_create(&certificate, &key).unwrap();
    assert!(!created);
    assert_eq!(again.fingerprint, first.fingerprint);

    // A lost certificate is remade from the key: still the same server.
    let old_certificate = std::fs::read(&certificate).unwrap();
    std::fs::remove_file(&certificate).unwrap();
    let (remade, created) = tls::load_or_create(&certificate, &key).unwrap();
    assert!(!created);
    assert_eq!(remade.fingerprint, first.fingerprint);
    assert_ne!(std::fs::read(&certificate).unwrap(), old_certificate, "a new certificate, the same key");

    // A lost key is not quietly replaced: that would be a different server.
    std::fs::remove_file(&key).unwrap();
    assert_eq!(tls::load_or_create(&certificate, &key).err(), Some(TlsError::KeyMissing));
    assert!(!key.exists());
}

#[tokio::test]
async fn an_operators_own_certificate_and_key_are_used_as_given() {
    let scratch = Scratch::new();
    let (certificate, key) = (scratch.0.join("fullchain.pem"), scratch.0.join("privkey.pem"));
    let made = rcgen::generate_simple_self_signed(vec!["chat.example.org".to_owned()]).unwrap();
    std::fs::write(&certificate, made.cert.pem()).unwrap();
    std::fs::write(&key, made.signing_key.serialize_pem()).unwrap();

    let identity = tls::load(&certificate, &key).unwrap();
    assert_eq!(Some(identity.fingerprint), deaddrop_tls::fingerprint(made.cert.der()));

    assert_eq!(tls::load(&scratch.0.join("missing.pem"), &key).err(), Some(TlsError::Read));
    assert_eq!(tls::load(&certificate, &scratch.0.join("missing.pem")).err(), Some(TlsError::Read));
    std::fs::write(scratch.0.join("empty.pem"), "").unwrap();
    assert_eq!(tls::load(&scratch.0.join("empty.pem"), &key).err(), Some(TlsError::NoCertificate));
    assert_eq!(tls::load(&certificate, &scratch.0.join("empty.pem")).err(), Some(TlsError::BadKey));
}

#[tokio::test]
async fn a_client_that_does_not_speak_tls_gets_nothing() {
    let scratch = Scratch::new();
    let running = start(&scratch.0).await;
    let mut stream = TcpStream::connect(running.address).await.unwrap();
    stream.write_all(b"GET /v1/server HTTP/1.1\r\nHost: deaddrop\r\nConnection: close\r\n\r\n").await.unwrap();
    let mut response = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(15), stream.read_to_end(&mut response)).await;
    let response = String::from_utf8_lossy(&response);
    assert!(!response.contains("HTTP/1.1"), "no HTTP answer over an unencrypted connection");
    assert!(!response.contains("server_id"));
}

#[tokio::test]
async fn live_delivery_works_over_tls() {
    let scratch = Scratch::new();
    let running = start(&scratch.0).await;
    let server = &running.server;
    let (owner, member) = (server.phone().await, server.phone().await);
    let channel = server.channel_with(&owner, 3600, &[(&member, "Member")]).await;

    let stream = connect(running.address, &running.fingerprint).await.unwrap();
    let (mut socket, _) = tokio_tungstenite::client_async("wss://deaddrop/v1/events", stream).await.unwrap();
    socket.send(Message::text(json!({ "token": member.token }).to_string())).await.unwrap();
    let mut next = async || loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), socket.next()).await.expect("a frame in time");
        if let Message::Text(text) = frame.expect("an open connection").unwrap() {
            return serde_json::from_str::<Value>(text.as_str()).unwrap();
        }
    };
    assert_eq!(next().await, json!({ "type": "ready" }));
    server.send(&owner, &channel, "pushed over tls").await;
    let pushed = next().await;
    assert_eq!((&pushed["type"], &pushed["body"]), (&json!("message"), &json!(encoded("pushed over tls"))));
}

#[tokio::test]
async fn the_wrong_password_limit_sees_callers_through_tls() {
    let scratch = Scratch::new();
    let running = start(&scratch.0).await;
    let server = &running.server;
    let set = json!({ "password": "correct horse" });
    assert_eq!(
        server.as_phone(&server.operator, PUT, "/v1/operator/password", Some(set)).await.0,
        StatusCode::NO_CONTENT
    );

    let attempt = async |password: &str| {
        let mut body = registration(&SoftDevice::new());
        body["password"] = json!(password);
        let stream = connect(running.address, &running.fingerprint).await.unwrap();
        request(stream, "POST", "/v1/devices", Some(body)).await.0
    };
    for _ in 0..10 {
        assert_eq!(attempt("wrong").await, 403);
    }
    assert_eq!(attempt("correct horse").await, 429);
    assert_eq!(server.state.failures.lock().unwrap().remembered(), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn the_key_file_is_readable_only_by_its_owner() {
    use std::os::unix::fs::PermissionsExt;
    let scratch = Scratch::new();
    let key = scratch.0.join("key.pem");
    tls::load_or_create(&scratch.0.join("cert.pem"), &key).unwrap();
    assert_eq!(std::fs::metadata(&key).unwrap().permissions().mode() & 0o777, 0o600);
}

/// Answers every connection with a fixed certificate and key, without checking that
/// they belong together: what an attacker's own TLS server can do.
#[derive(Debug)]
struct Whatever(Arc<rustls::sign::CertifiedKey>);

impl rustls::server::ResolvesServerCert for Whatever {
    fn resolve(&self, _hello: rustls::server::ClientHello<'_>) -> Option<Arc<rustls::sign::CertifiedKey>> {
        Some(self.0.clone())
    }
}

#[tokio::test]
async fn showing_the_real_certificate_is_not_enough_without_its_key() {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};

    // The real server's certificate is public: anyone who connects is sent it.
    let real_files = Scratch::new();
    let real = start(&real_files.0).await;
    let real_certificate = CertificateDer::from_pem_file(real_files.0.join("cert.pem")).unwrap();
    assert_eq!(deaddrop_tls::fingerprint(&real_certificate).as_deref(), Some(real.fingerprint.as_str()));

    // The attacker presents that certificate, but can only sign with a key of their own.
    let own_key = rcgen::KeyPair::generate().unwrap();
    let own_key = PrivateKeyDer::from_pem_slice(own_key.serialize_pem().as_bytes()).unwrap();
    let signer = deaddrop_tls::provider().key_provider.load_private_key(own_key).unwrap();
    let forged = rustls::sign::CertifiedKey::new(vec![real_certificate], signer);
    let config = rustls::ServerConfig::builder_with_provider(deaddrop_tls::provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_cert_resolver(Arc::new(Whatever(Arc::new(forged))));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        while let Ok((stream, _)) = listener.accept().await {
            let _ = acceptor.accept(stream).await;
        }
    });

    // The certificate has the right fingerprint, so only the proof of holding the key
    // stands between the attacker and the phone. It holds.
    let refused = connect(address, &real.fingerprint).await;
    assert!(refused.is_err(), "a phone must not accept a server that cannot prove it holds the key");
    assert!(connect(real.address, &real.fingerprint).await.is_ok());
}
