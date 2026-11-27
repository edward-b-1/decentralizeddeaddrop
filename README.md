# Decentralized Dead Drop

*Secure communication systems.*

A text messenger where every community runs its own server. The server is a dead drop: it
holds sealed messages only until they have been collected, and can never read them.

**Status: early development.** The server, the encryption and the Android app exist
and work together on an emulator; nothing has run on a real phone yet. It has had no
independent security review. Do not rely on it yet.

- `docs/PROTOCOL.md`: the design, and what is and is not built.
- `docs/DEPLOYMENT.md`: running your own server (files in `deploy/`).
- `docs/PLAY_STORE.md`: what a Play Store release needs.
- `crates/core`: formats shared by the server, the command-line client and the app.
- `crates/client`: the client engine the app uses, and `deaddrop`, a command-line client built on it.
- `crates/crypto`: the end-to-end encryption, built on the vodozemac library.
- `crates/tls`: the server fingerprint, and the check a phone makes with it.
- `crates/ffi`: the bridge that lets the Android app call the client engine.
- `android/`: the Android app.
- `crates/server`: the server, `deaddropd`. See its README for running it and the API.

## Building

Needs Rust 1.100 or later and a C compiler (for the bundled SQLite).

```
cargo test --workspace
cargo run -p deaddropd
```

## Building for Android

The app uses the Rust crates as native code. That needs the Android NDK, Rust's
Android targets and `cargo-ndk` (then see `android/README.md`):

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

## Licence

[PolyForm Noncommercial License 1.0.0](LICENSE.md). You may use, change and share this
software for any noncommercial purpose. That covers personal use, and use by charities,
schools, public research bodies and government. Commercial use needs the author's
permission.

The source is public so that anyone can check it, but this is not an open-source
licence: describe the project as "source available".

Required Notice: Copyright 2026 the author of Decentralized Dead Drop (https://decentralizeddeaddrop.com)

The libraries it is built on keep their own licences, mostly MIT and Apache 2.0.
