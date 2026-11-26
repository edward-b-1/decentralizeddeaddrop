//! Turning a [`Messenger`] into bytes and back, for the app to keep between runs.

use serde::{Deserialize, Serialize};
use vodozemac::megolm::{GroupSession, GroupSessionPickle, InboundGroupSession, InboundGroupSessionPickle};
use vodozemac::olm::{Account, AccountPickle, Session, SessionPickle};
use vodozemac::{Curve25519PublicKey, Ed25519PublicKey};

use crate::{Channel, Messenger, Outbound, Peer};

#[derive(Serialize, Deserialize)]
struct Saved {
    v: u32,
    device: String,
    account: AccountPickle,
    peers: Vec<SavedPeer>,
    channels: Vec<SavedChannel>,
}

#[derive(Serialize, Deserialize)]
struct SavedPeer {
    device: String,
    curve25519: String,
    ed25519: String,
    sessions: Vec<SessionPickle>,
}

#[derive(Serialize, Deserialize)]
struct SavedChannel {
    id: String,
    roster_version: u64,
    members: Vec<String>,
    outbound: Option<SavedOutbound>,
    inbound: Vec<(String, Vec<InboundGroupSessionPickle>)>,
}

#[derive(Serialize, Deserialize)]
struct SavedOutbound {
    session: GroupSessionPickle,
    roster_version: u64,
    created: i64,
    shared_with: Vec<String>,
}

pub fn save(messenger: &Messenger) -> Vec<u8> {
    let saved = Saved {
        v: 1,
        device: messenger.device.clone(),
        account: messenger.account.pickle(),
        peers: messenger
            .peers
            .iter()
            .map(|(device, peer)| SavedPeer {
                device: device.clone(),
                curve25519: peer.curve25519.to_base64(),
                ed25519: peer.ed25519.to_base64(),
                sessions: peer.sessions.iter().map(Session::pickle).collect(),
            })
            .collect(),
        channels: messenger
            .channels
            .iter()
            .map(|(id, channel)| SavedChannel {
                id: id.clone(),
                roster_version: channel.roster_version,
                members: channel.members.iter().cloned().collect(),
                outbound: channel.outbound.as_ref().map(|outbound| SavedOutbound {
                    session: outbound.session.pickle(),
                    roster_version: outbound.roster_version,
                    created: outbound.created,
                    shared_with: outbound.shared_with.iter().cloned().collect(),
                }),
                inbound: channel
                    .inbound
                    .iter()
                    .map(|(sender, held)| (sender.clone(), held.iter().map(InboundGroupSession::pickle).collect()))
                    .collect(),
            })
            .collect(),
    };
    serde_json::to_vec(&saved).expect("plain data")
}

pub fn load(bytes: &[u8]) -> Option<Messenger> {
    let saved: Saved = serde_json::from_slice(bytes).ok()?;
    if saved.v != 1 {
        return None;
    }
    let mut peers = std::collections::HashMap::new();
    for peer in saved.peers {
        peers.insert(
            peer.device,
            Peer {
                curve25519: Curve25519PublicKey::from_base64(&peer.curve25519).ok()?,
                ed25519: Ed25519PublicKey::from_base64(&peer.ed25519).ok()?,
                sessions: peer.sessions.into_iter().map(Session::from_pickle).collect(),
            },
        );
    }
    let mut channels = std::collections::HashMap::new();
    for channel in saved.channels {
        channels.insert(
            channel.id,
            Channel {
                roster_version: channel.roster_version,
                members: channel.members.into_iter().collect(),
                outbound: channel.outbound.map(|outbound| Outbound {
                    session: GroupSession::from_pickle(outbound.session),
                    roster_version: outbound.roster_version,
                    created: outbound.created,
                    shared_with: outbound.shared_with.into_iter().collect(),
                }),
                inbound: channel
                    .inbound
                    .into_iter()
                    .map(|(sender, held)| (sender, held.into_iter().map(InboundGroupSession::from_pickle).collect()))
                    .collect(),
            },
        );
    }
    Some(Messenger { device: saved.device, account: Account::from_pickle(saved.account), peers, channels })
}
