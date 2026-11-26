//! The open connection over which a server says "there is something for you".
//!
//! Nothing is read from it but that nudge: on hearing one, the app collects its queue
//! the ordinary way ([`crate::Account::sync`]). So a lost or late nudge can never lose
//! a message, only delay it until the next sync.

use std::io::ErrorKind;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConnection, StreamOwned};
use serde_json::{Value, json};
use tungstenite::{Message, WebSocket};

use crate::ClientError;
use crate::link::ServerAddress;

pub struct Live {
    socket: WebSocket<StreamOwned<ClientConnection, TcpStream>>,
}

/// What [`Live::wait`] came back with.
#[derive(Debug, PartialEq, Eq)]
pub enum Wake {
    /// A message is waiting, or a channel changed: time to sync.
    Sync,
    /// Nothing happened within the time given.
    Quiet,
}

impl Live {
    pub(crate) fn connect(address: &ServerAddress, session_token: &str) -> Result<Live, ClientError> {
        let config = deaddrop_tls::pinned_client_config(&address.fingerprint).ok_or(ClientError::BadLink)?;
        let host = address.host.trim_start_matches('[').trim_end_matches(']');
        let target = (host, address.port).to_socket_addrs().ok().and_then(|mut found| found.next());
        let target = target.ok_or(ClientError::Unreachable)?;
        let tcp = TcpStream::connect_timeout(&target, Duration::from_secs(10)).map_err(|_| ClientError::Unreachable)?;
        tcp.set_read_timeout(Some(Duration::from_secs(20))).map_err(|_| ClientError::Unreachable)?;

        // The name is not checked (the fingerprint is), but TLS wants one.
        let name = ServerName::try_from(host.to_owned()).map_err(|_| ClientError::BadLink)?;
        let tls = ClientConnection::new(Arc::new(config), name).map_err(|_| ClientError::Unreachable)?;
        let url = format!("wss://{}/v1/events", address.authority());
        let (mut socket, _) =
            tungstenite::client(url, StreamOwned::new(tls, tcp)).map_err(|_| ClientError::Unreachable)?;

        socket
            .send(Message::text(json!({ "token": session_token }).to_string()))
            .map_err(|_| ClientError::Unreachable)?;
        match read_json(&mut socket)? {
            Some(frame) if frame["type"] == "ready" => Ok(Live { socket }),
            _ => Err(ClientError::Refused { status: 401, message: "sign in first".into() }),
        }
    }

    /// Waits up to `timeout` for the server to say something. An error means the
    /// connection has gone and a new one is needed.
    pub fn wait(&mut self, timeout: Duration) -> Result<Wake, ClientError> {
        self.socket
            .get_ref()
            .get_ref()
            .set_read_timeout(Some(timeout.max(Duration::from_millis(1))))
            .map_err(|_| ClientError::Unreachable)?;
        Ok(match read_json(&mut self.socket)? {
            Some(frame) if frame["type"] == "message" || frame["type"] == "changed" => Wake::Sync,
            _ => Wake::Quiet,
        })
    }
}

/// The next JSON frame; `None` if the read timed out or the frame was not JSON text
/// (keepalive pings are answered by the library).
fn read_json(socket: &mut WebSocket<StreamOwned<ClientConnection, TcpStream>>) -> Result<Option<Value>, ClientError> {
    match socket.read() {
        Ok(Message::Text(text)) => Ok(serde_json::from_str(text.as_str()).ok()),
        Ok(Message::Close(_)) => Err(ClientError::Unreachable),
        Ok(_) => Ok(None),
        Err(tungstenite::Error::Io(error)) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
            Ok(None)
        }
        Err(_) => Err(ClientError::Unreachable),
    }
}
