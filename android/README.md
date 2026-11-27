# The Android app

Kotlin and Compose, Android 12 or later, app id `com.decentralizeddeaddrop`. The screens
follow `docs/design/app-2026-11-26-a.html`.

The app itself only draws screens, holds the hardware keys and stores files. Everything
else (talking to servers, checking what they say, encryption) is the Rust engine in
`crates/client`, reached through the bridge in `crates/ffi`.

## Building

1. Once, and again whenever the Rust code changes, build the engine and its Kotlin
   bindings (needs the NDK set up as in the README at the top of the repo):

   ```
   ANDROID_NDK_HOME=<path to the NDK> sh android/build-engine.sh          # or: ... build-engine.sh debug
   ```

   This writes `app/src/main/jniLibs/` and `app/src/main/java/.../engine/`. Both are
   generated and not kept in git.
2. Build the app: `./gradlew :app:assembleDebug` (from `android/`).

On the emulator, a server running on this computer is reached at `10.0.2.2`: start it
with `deaddropd --public 10.0.2.2:7443` so the links it prints work from the emulator.

## How it is put together

| File | Job |
|---|---|
| `keys/Keys.kt` | `DeviceKeys`: an identity's two long-term keys in the Android Keystore, one identity per server. `Vault`: files encrypted with a Keystore key (engine state, message history). |
| `data/Servers.kt` | Every server the phone is on: its engine session, a background thread that collects messages and stores each one before telling the engine it may forget it, and what the screens show. |
| `ui/` | The screens. `DeadDropUi.kt` holds the tabs and navigation. |
| `ui/Qr.kt`, `ui/QrCodes.kt` | QR codes, made and read on the phone (ZXing, with CameraX for the camera). No Google service is involved. |
| `data/MessageDb.kt` | Message history: one database for every server, encrypted as a whole with SQLCipher. Its key is random, kept sealed by the Keystore key. A conversation is read 200 messages at a time. A message from a server is stored with the engine's ID for it, so one handed over twice is stored once. |
| `res/drawable/ic_launcher_*.xml`, `ic_notification.xml` | The icon: a message bubble folded like a sealed envelope, in front of a second bubble. Three vector files (home screen, one-colour themed, status bar), all drawn from `docs/design/icon-concepts-blue-grey-2026-11-27/sealed-conversation.svg`. |
| `ui/Licences.kt` | Settings, Licences: this app's licence and those of every library built in. The text is `assets/licences.txt`, written by `python tools/licences.py`; run that again when a dependency changes. |
| `service/Background.kt` | Staying connected while the app is closed: the foreground service, notifications, the receiver that reconnects after a restart, and the two settings. |

Opening a `deaddrop://` link starts the app on the right screen (add a server, or join a
channel).

## Release builds

`./gradlew bundleRelease` makes a shrunk App Bundle, signed if `keystore.properties`
exists here (git-ignored; `docs/PLAY_STORE.md` says what goes in it). Build the engine
first **without** `debug`: a debug engine makes the app several hundred megabytes.

## Tests

`./gradlew testDebugUnitTest` runs the QR code tests on this computer. Everything else in
the app is screens over the Rust engine, which has its own tests (`cargo test --workspace`).

## Not built yet

- Version 0.1.0 has run on one real phone. Two phones in one channel, and staying
  connected through a night on mobile data, are the things most in need of trying.

`docs/TODO.md` has the full list, and what has been decided against.
