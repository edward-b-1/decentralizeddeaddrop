# Decentralized Dead Drop

*Secure communication systems.*

A text messenger where every community runs its own server. The server is a dead drop: it
holds sealed messages only until they have been collected, and can never read them.

**Status: early development.** The server and the message encryption exist; the Android
app does not. It has had no independent security review. Do not rely on it yet.

- `docs/PROTOCOL.md`: the design, and what is and is not built.
- `crates/core`: formats shared by the server, test clients and (later) the app.
- `crates/crypto`: the end-to-end encryption, built on the vodozemac library.
- `crates/tls`: the server fingerprint, and the check a phone makes with it.
- `crates/server`: the server, `deaddropd`. See its README for running it and the API.

## Building

Needs Rust 1.100 or later and a C compiler (for the bundled SQLite).

```
cargo test --workspace
cargo run -p deaddropd
```
