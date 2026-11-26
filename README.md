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

## Building for Android

The app will use `crates/core`, `crates/crypto` and `crates/tls` as native code. That
needs the Android NDK, Rust's Android targets and `cargo-ndk`:

```
sdkmanager --install "ndk;30.0.16248370"
rustup target add aarch64-linux-android x86_64-linux-android
cargo install cargo-ndk
```

Then, with `ANDROID_NDK_HOME` pointing at the NDK folder:

```
cargo ndk -t arm64-v8a -t x86_64 --platform 31 build -p deaddrop-core -p deaddrop-crypto -p deaddrop-tls
```

`arm64-v8a` is for phones and `x86_64` for the emulator. `cargo ndk ... test --no-run`
builds the tests as Android programs, which can be pushed to a device with `adb` and run.
