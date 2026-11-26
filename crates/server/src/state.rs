//! What every handler shares.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use deaddrop_core::encoding::{random_id, random_token};
use tokio::sync::broadcast;

use crate::error::ApiResult;
use crate::logging::FailureLog;
use crate::store::{self, Store};

/// Unix time in seconds. Tests move it forward instead of sleeping.
#[derive(Clone, Default)]
pub struct Clock {
    offset: Arc<AtomicI64>,
}

impl Clock {
    pub fn now(&self) -> i64 {
        let system = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        system + self.offset.load(Ordering::Relaxed)
    }

    pub fn advance(&self, seconds: i64) {
        self.offset.fetch_add(seconds, Ordering::Relaxed);
    }
}

/// Why a connected device is being nudged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wake {
    /// A message is waiting in its queue.
    Message,
    /// A channel, roster or join request it can see has changed.
    Changed,
}

/// Where a request came from, as far as the password limit cares. An IPv6 caller is its
/// /64 network, since one subscriber usually has a whole /64 of addresses to guess from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Caller {
    V4(Ipv4Addr),
    V6Network(u64),
    /// The address is not known (it never is in tests that skip the network).
    Unknown,
}

impl Caller {
    pub fn of(address: Option<IpAddr>) -> Caller {
        match address.map(|address| address.to_canonical()) {
            Some(IpAddr::V4(v4)) => Caller::V4(v4),
            Some(IpAddr::V6(v6)) => Caller::V6Network((u128::from(v6) >> 64) as u64),
            None => Caller::Unknown,
        }
    }
}

/// Wrong server passwords, counted per network address, so someone guessing is stopped
/// without locking anyone else out.
///
/// This is the one place the server looks at an address. The count is held in memory
/// only and forgotten ten minutes after the last wrong guess from that address; each
/// wrong guess is also logged for firewall tools (`logging::password_line`). A caller
/// who gets the password right is never recorded anywhere.
#[derive(Default)]
pub struct Failures {
    by_caller: HashMap<Caller, VecDeque<i64>>,
}

impl Failures {
    pub const LIMIT: usize = 10;
    pub const WINDOW: i64 = 600;

    /// Whether `caller` has used up their wrong guesses for now.
    pub fn blocked(&mut self, caller: Caller, now: i64) -> bool {
        let Some(times) = self.by_caller.get_mut(&caller) else { return false };
        while times.front().is_some_and(|&time| time <= now - Self::WINDOW) {
            times.pop_front();
        }
        if times.is_empty() {
            self.by_caller.remove(&caller);
            return false;
        }
        times.len() >= Self::LIMIT
    }

    pub fn record(&mut self, caller: Caller, now: i64) {
        self.by_caller.entry(caller).or_default().push_back(now);
    }

    /// Drops every caller whose wrong guesses are all older than the window.
    pub fn forget_old(&mut self, now: i64) {
        self.by_caller.retain(|_, times| times.back().is_some_and(|&time| time > now - Self::WINDOW));
    }

    /// How many callers are remembered right now.
    pub fn remembered(&self) -> usize {
        self.by_caller.len()
    }
}

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub clock: Clock,
    /// A random name this server gave itself; devices sign it into every sign-in.
    pub server_id: Arc<str>,
    pub failures: Arc<Mutex<Failures>>,
    /// Where wrong server passwords are logged, with the caller's address.
    pub failure_log: FailureLog,
    wake: broadcast::Sender<(Arc<str>, Wake)>,
}

impl AppState {
    /// Prepares a server on `store`. On a database with no operator yet, also returns a
    /// new one-time operator token: the first device to register with it runs the server.
    pub async fn open(store: Store) -> ApiResult<(AppState, Option<String>)> {
        let (server_id, operator_token) = store
            .run(|db| {
                let server_id = match store::get_meta(db, "server_id")? {
                    Some(id) => id,
                    None => {
                        let id = random_id();
                        store::set_meta(db, "server_id", &id)?;
                        id
                    }
                };
                let has_operator: bool =
                    db.query_row("SELECT EXISTS (SELECT 1 FROM devices WHERE operator = 1)", [], |row| row.get(0))?;
                let operator_token = if has_operator { None } else { Some(new_operator_token(db)?) };
                Ok((server_id, operator_token))
            })
            .await?;
        let (wake, _) = broadcast::channel(1024);
        let state = AppState {
            store,
            clock: Clock::default(),
            server_id: server_id.into(),
            failures: Arc::default(),
            failure_log: FailureLog::default(),
            wake,
        };
        Ok((state, operator_token))
    }

    pub fn wake(&self, device: &str, kind: Wake) {
        // An error only means nobody is connected right now.
        let _ = self.wake.send((device.into(), kind));
    }

    pub fn wake_all(&self, devices: &[String], kind: Wake) {
        for device in devices {
            self.wake(device, kind);
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<(Arc<str>, Wake)> {
        self.wake.subscribe()
    }
}

/// Replaces the operator token with a new one; only its hash is stored.
pub fn new_operator_token(db: &rusqlite::Connection) -> rusqlite::Result<String> {
    let token = random_token();
    let hash = store::sha256(&token).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    store::set_meta(db, "operator_token", &hash)?;
    Ok(token)
}

/// Whether `token` is the unspent operator token.
pub fn is_operator_token(db: &rusqlite::Connection, token: &str) -> rusqlite::Result<bool> {
    let hash = store::sha256(token).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    Ok(store::get_meta(db, "operator_token")?.is_some_and(|stored| stored == hash))
}
