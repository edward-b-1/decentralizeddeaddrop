//! `deaddrop://` links: what a QR code or a pasted invite holds.
//!
//! ```text
//! deaddrop://HOST:PORT/#fp=<fingerprint>[&t=<server token>][&c=<channel>&o=<owner>&ct=<channel token>]
//! ```
//!
//! - `fp`: the server's fingerprint. With it the app recognises the server by itself.
//! - `t`: a token that lets a new device onto the server.
//! - `c`, `o`, `ct`: an invite to a channel: its ID, its owner's device ID, and a token
//!   that lets the holder ask to join. The owner's ID is what lets the joiner check that
//!   the channel they are shown really is the one they were invited to.
//!
//! Everything after `#` stays on the phone: it is never part of any request.

use deaddrop_core::encoding::{ID_LEN, TOKEN_LEN, is_base32};
use deaddrop_core::keys::DEVICE_ID_LEN;
use serde::{Deserialize, Serialize};

/// Where a server is and how to recognise it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerAddress {
    /// A hostname or IP address (IPv6 in square brackets).
    pub host: String,
    pub port: u16,
    /// The server's fingerprint, 52 base32 characters.
    pub fingerprint: String,
}

impl ServerAddress {
    pub fn authority(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// An invite to one channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelInvite {
    pub channel: String,
    pub owner: String,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub server: ServerAddress,
    pub server_token: Option<String>,
    pub channel: Option<ChannelInvite>,
}

impl Link {
    /// Reads a link. `None` for anything that is not a complete, well-formed one.
    pub fn parse(text: &str) -> Option<Link> {
        let rest = text.trim().strip_prefix("deaddrop://")?;
        let (authority, fragment) = rest.split_once("/#")?;
        let (host, port) = authority.rsplit_once(':')?;
        let port: u16 = port.parse().ok()?;
        let host_ok = !host.is_empty()
            && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
        if !host_ok {
            return None;
        }

        let (mut fingerprint, mut server_token) = (None, None);
        let (mut channel, mut owner, mut channel_token) = (None, None, None);
        for pair in fragment.split('&') {
            let (key, value) = pair.split_once('=')?;
            let slot = match key {
                "fp" => &mut fingerprint,
                "t" => &mut server_token,
                "c" => &mut channel,
                "o" => &mut owner,
                "ct" => &mut channel_token,
                // Unknown parts are ignored so that newer links still open.
                _ => continue,
            };
            *slot = Some(value.to_owned());
        }
        let fingerprint = deaddrop_tls::parse(&fingerprint?)?;
        if server_token.as_ref().is_some_and(|token| !is_base32(token, TOKEN_LEN)) {
            return None;
        }
        let channel = match (channel, owner, channel_token) {
            (None, None, None) => None,
            (Some(channel), Some(owner), Some(token))
                if is_base32(&channel, ID_LEN) && is_base32(&owner, DEVICE_ID_LEN) && is_base32(&token, TOKEN_LEN) =>
            {
                Some(ChannelInvite { channel, owner, token })
            }
            _ => return None,
        };
        Some(Link { server: ServerAddress { host: host.to_owned(), port, fingerprint }, server_token, channel })
    }
}

impl std::fmt::Display for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "deaddrop://{}/#fp={}", self.server.authority(), self.server.fingerprint)?;
        if let Some(token) = &self.server_token {
            write!(f, "&t={token}")?;
        }
        if let Some(invite) = &self.channel {
            write!(f, "&c={}&o={}&ct={}", invite.channel, invite.owner, invite.token)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "wmcchgovezu6vnuyxkuy72o3bscfb64dqgl7xp4hbptm5rfnyqiq";
    const TOKEN: &str = "abcdefghijklmnopqrstuvwxyz234567";
    const CHANNEL: &str = "abcdefghijklmnopqrstuvwxyz";
    const OWNER: &str = "t2n6vrwrnowcmdaw";

    #[test]
    fn a_server_link_round_trips() {
        let text = format!("deaddrop://203.0.113.7:7443/#fp={FP}&t={TOKEN}");
        let link = Link::parse(&text).unwrap();
        assert_eq!(link.server, ServerAddress { host: "203.0.113.7".into(), port: 7443, fingerprint: FP.into() });
        assert_eq!(link.server_token.as_deref(), Some(TOKEN));
        assert_eq!(link.channel, None);
        assert_eq!(link.to_string(), text);
    }

    #[test]
    fn a_channel_invite_round_trips() {
        let text = format!("deaddrop://chat.example.org:443/#fp={FP}&c={CHANNEL}&o={OWNER}&ct={TOKEN}");
        let link = Link::parse(&text).unwrap();
        let invite = link.channel.clone().unwrap();
        assert_eq!((invite.channel.as_str(), invite.owner.as_str(), invite.token.as_str()), (CHANNEL, OWNER, TOKEN));
        assert_eq!(link.server_token, None);
        assert_eq!(link.to_string(), text);
        // IPv6 addresses, surrounding spaces and unknown parts are all fine.
        let v6 = format!("  deaddrop://[2001:db8::7]:7443/#fp={FP}&future=1 ");
        assert_eq!(Link::parse(&v6).unwrap().server.host, "[2001:db8::7]");
    }

    #[test]
    fn incomplete_or_malformed_links_are_refused() {
        for bad in [
            String::new(),
            "https://203.0.113.7:7443/".into(),
            "deaddrop://203.0.113.7:7443/".into(),
            "deaddrop://203.0.113.7:7443/#t=abc".into(),
            format!("deaddrop://203.0.113.7/#fp={FP}"),
            format!("deaddrop://203.0.113.7:99999/#fp={FP}"),
            format!("deaddrop://evil.example/path@203.0.113.7:7443/#fp={FP}"),
            format!("deaddrop://203.0.113.7:7443/#fp={}", &FP[1..]),
            format!("deaddrop://203.0.113.7:7443/#fp={FP}&t=short"),
            // A channel invite needs all three of its parts.
            format!("deaddrop://203.0.113.7:7443/#fp={FP}&c={CHANNEL}&ct={TOKEN}"),
            format!("deaddrop://203.0.113.7:7443/#fp={FP}&c={CHANNEL}&o=not-an-id&ct={TOKEN}"),
        ] {
            assert_eq!(Link::parse(&bad), None, "{bad}");
        }
    }
}
