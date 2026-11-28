//! The SQLite database: schema and the small queries several handlers share.

use std::path::Path;
use std::sync::{Arc, Mutex};

use deaddrop_core::encoding::random_token;
use deaddrop_core::records::{Leave, Signed};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{ApiError, ApiResult};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS devices (
    id            TEXT PRIMARY KEY,
    signing_key   BLOB NOT NULL,
    agreement_key BLOB NOT NULL,
    agreement_sig BLOB NOT NULL,
    status        TEXT NOT NULL CHECK (status IN ('active', 'pending')),
    operator      INTEGER NOT NULL DEFAULT 0
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS challenges (
    challenge TEXT PRIMARY KEY,
    device    TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    expires   INTEGER NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS sessions (
    token_hash BLOB PRIMARY KEY,
    device     TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    expires    INTEGER NOT NULL
) WITHOUT ROWID;

-- A device's messaging keys (a record signed by the device) and its prekeys. All
-- public keys: the server hands them to other members and can do nothing else with them.
CREATE TABLE IF NOT EXISTS messaging_keys (
    device    TEXT PRIMARY KEY REFERENCES devices(id) ON DELETE CASCADE,
    record    TEXT NOT NULL,
    signature TEXT NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS prekeys (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    device    TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    key       TEXT NOT NULL,
    signature TEXT NOT NULL,
    fallback  INTEGER NOT NULL DEFAULT 0,
    UNIQUE (device, key)
);

-- Invite tokens. scope is 'server' (a way in to the server) or a channel ID.
CREATE TABLE IF NOT EXISTS tokens (
    hash      BLOB PRIMARY KEY,
    scope     TEXT NOT NULL,
    uses_left INTEGER NOT NULL,
    expires   INTEGER NOT NULL
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS channels (
    id             TEXT PRIMARY KEY,
    owner          TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    record         TEXT NOT NULL,
    signature      TEXT NOT NULL,
    version        INTEGER NOT NULL,
    max_age        INTEGER NOT NULL,
    roster         TEXT NOT NULL,
    roster_sig     TEXT NOT NULL,
    roster_version INTEGER NOT NULL
) WITHOUT ROWID;

-- Who messages are delivered to: the devices in the owner's current roster, less any
-- that have left since it was signed (gone = 1).
CREATE TABLE IF NOT EXISTS members (
    channel TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    device  TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    gone    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (channel, device)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS members_by_device ON members(device);

-- A member's signed word that they have left a channel (records::Leave), for the owner's
-- phone to check. Kept until the owner's roster drops the member. It holds the signing
-- key and has no link to the devices table because the device itself may be deleted by
-- then: taking a server off a phone leaves these behind for the channels it was in.
CREATE TABLE IF NOT EXISTS leaves (
    channel     TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    device      TEXT NOT NULL,
    signing_key BLOB NOT NULL,
    record      TEXT NOT NULL,
    signature   TEXT NOT NULL,
    PRIMARY KEY (channel, device)
) WITHOUT ROWID;

CREATE TABLE IF NOT EXISTS join_requests (
    channel TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    device  TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    name    TEXT NOT NULL,
    PRIMARY KEY (channel, device)
) WITHOUT ROWID;

-- The delivery queue: one sealed copy per message, and one pending mark per recipient.
-- A message row exists only while at least one pending mark does.
CREATE TABLE IF NOT EXISTS messages (
    seq     INTEGER PRIMARY KEY AUTOINCREMENT,
    id      TEXT NOT NULL UNIQUE,
    channel TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    sender  TEXT NOT NULL,
    body    BLOB NOT NULL,
    created INTEGER NOT NULL,
    expires INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS messages_by_expiry ON messages(expires);
CREATE INDEX IF NOT EXISTS messages_by_channel ON messages(channel);

CREATE TABLE IF NOT EXISTS pending (
    device TEXT NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    seq    INTEGER NOT NULL REFERENCES messages(seq) ON DELETE CASCADE,
    PRIMARY KEY (device, seq)
) WITHOUT ROWID;
CREATE INDEX IF NOT EXISTS pending_by_seq ON pending(seq);
";

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    pub fn open(path: &Path) -> rusqlite::Result<Store> {
        let conn = Connection::open(path)?;
        // Write-ahead logging keeps readers and the writer out of each other's way.
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        Store::prepare(conn)
    }

    pub fn in_memory() -> rusqlite::Result<Store> {
        Store::prepare(Connection::open_in_memory()?)
    }

    fn prepare(conn: Connection) -> rusqlite::Result<Store> {
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        // Overwrite deleted rows with zeros instead of only unlinking them.
        conn.query_row("PRAGMA secure_delete = ON", [], |_| Ok(()))?;
        conn.execute_batch(SCHEMA)?;
        Ok(Store { conn: Arc::new(Mutex::new(conn)) })
    }

    /// Runs `work` against the database on a blocking thread.
    pub async fn run<T, F>(&self, work: F) -> ApiResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> ApiResult<T> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = conn.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            work(&mut conn)
        })
        .await
        .unwrap_or_else(|_| Err(ApiError::internal()))
    }
}

pub fn sha256(text: &str) -> Vec<u8> {
    Sha256::digest(text.as_bytes()).to_vec()
}

pub fn get_meta(db: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    db.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| row.get(0)).optional()
}

pub fn set_meta(db: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    db.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = ?2",
        [key, value],
    )?;
    Ok(())
}

pub fn delete_meta(db: &Connection, key: &str) -> rusqlite::Result<()> {
    db.execute("DELETE FROM meta WHERE key = ?1", [key])?;
    Ok(())
}

/// Which ways in the server accepts (docs/PROTOCOL.md section 3).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Settings {
    /// Anyone who can reach the server may register.
    pub open: bool,
    /// Invite tokens made by the operator are accepted.
    pub tokens: bool,
    /// New devices wait for the operator's approval, whichever way they came in.
    pub approval: bool,
    /// Argon2id hash of the server password, if one is set.
    #[serde(default)]
    pub password_hash: Option<String>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { open: false, tokens: true, approval: false, password_hash: None }
    }
}

pub fn load_settings(db: &Connection) -> rusqlite::Result<Settings> {
    let text = get_meta(db, "settings")?;
    Ok(text.and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default())
}

pub fn save_settings(db: &Connection, settings: &Settings) -> rusqlite::Result<()> {
    set_meta(db, "settings", &serde_json::to_string(settings).expect("settings are plain data"))
}

/// Makes an invite token for `scope`; only its hash is stored.
pub fn create_token(db: &Connection, scope: &str, uses: i64, expires: i64) -> rusqlite::Result<String> {
    let token = random_token();
    db.execute(
        "INSERT INTO tokens (hash, scope, uses_left, expires) VALUES (?1, ?2, ?3, ?4)",
        params![sha256(&token), scope, uses, expires],
    )?;
    Ok(token)
}

/// Spends one use of an invite token; false if it is unknown, used up, expired or for
/// something else.
pub fn consume_token(db: &Connection, scope: &str, token: &str, now: i64) -> rusqlite::Result<bool> {
    let hash = sha256(token);
    let spent = db.execute(
        "UPDATE tokens SET uses_left = uses_left - 1
         WHERE hash = ?1 AND scope = ?2 AND uses_left > 0 AND expires > ?3",
        params![hash, scope, now],
    )?;
    db.execute("DELETE FROM tokens WHERE hash = ?1 AND uses_left <= 0", params![hash])?;
    Ok(spent == 1)
}

pub fn signing_key(db: &Connection, device: &str) -> rusqlite::Result<Option<Vec<u8>>> {
    db.query_row("SELECT signing_key FROM devices WHERE id = ?1", [device], |row| row.get(0)).optional()
}

/// Checks a leave that `device` has signed for `channel` and keeps it for the owner's
/// phone. False, and nothing kept, if it is not that device's signed word about that
/// channel.
pub fn keep_leave(
    db: &Connection,
    channel: &str,
    device: &str,
    signing_key: &[u8],
    signed: &Signed,
) -> rusqlite::Result<bool> {
    let Ok(leave) = signed.open::<Leave>(signing_key) else { return Ok(false) };
    if leave.channel != channel || leave.device != device {
        return Ok(false);
    }
    db.execute(
        "INSERT INTO leaves (channel, device, signing_key, record, signature) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (channel, device) DO UPDATE SET signing_key = ?3, record = ?4, signature = ?5",
        params![channel, device, signing_key, signed.record, signed.signature],
    )?;
    Ok(true)
}

/// Whether `device` currently receives `channel`'s messages.
pub fn is_member(db: &Connection, channel: &str, device: &str) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS (SELECT 1 FROM members WHERE channel = ?1 AND device = ?2 AND gone = 0)",
        [channel, device],
        |row| row.get(0),
    )
}

/// Devices that currently receive `channel`'s messages.
pub fn member_devices(db: &Connection, channel: &str) -> rusqlite::Result<Vec<String>> {
    let mut query = db.prepare("SELECT device FROM members WHERE channel = ?1 AND gone = 0")?;
    query.query_map([channel], |row| row.get(0))?.collect()
}

/// Drops whatever `device` was still waiting for in `channel`.
pub fn drop_pending(db: &Connection, channel: &str, device: &str) -> rusqlite::Result<()> {
    db.execute(
        "DELETE FROM pending WHERE device = ?1 AND seq IN (SELECT seq FROM messages WHERE channel = ?2)",
        [device, channel],
    )?;
    Ok(())
}

/// Wipes every message nobody is waiting for any more. Called after anything that can
/// remove a pending mark, so a delivered message never outlives its last recipient.
///
/// Inside a transaction; follow the commit with [`checkpoint`].
pub fn wipe_delivered(db: &Connection) -> rusqlite::Result<usize> {
    db.execute("DELETE FROM messages WHERE NOT EXISTS (SELECT 1 FROM pending WHERE pending.seq = messages.seq)", [])
}

/// Folds the write-ahead log back into the database file and truncates it. Until this
/// runs, a wiped row's old bytes are still in the main file and its zeros only in the
/// log, so it follows every wipe. Harmless on an in-memory database.
pub fn checkpoint(db: &Connection) -> rusqlite::Result<()> {
    db.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())).optional()?;
    Ok(())
}
