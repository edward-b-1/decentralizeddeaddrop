//! Formats shared by the Dead Drop server, the test client and (later) the Android app.
//!
//! Nothing in this crate talks to a network or a disk. See `docs/PROTOCOL.md`.

pub mod encoding;
pub mod keys;
pub mod records;
pub mod soft;
