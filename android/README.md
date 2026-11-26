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
| `data/Servers.kt` | Every server the phone is on: its engine session, a background thread that collects messages, and what the screens show. |
| `ui/` | The screens. `DeadDropUi.kt` holds the tabs and navigation. |

Opening a `deaddrop://` link starts the app on the right screen (add a server, or join a
channel).

## Not built yet

- Receiving while the app is closed (it collects messages only while it is running).
- Scanning and showing QR codes, and adding a server by typing its address.
- Renaming a channel, changing your name in one, safety numbers.
- A proper message database (history is one encrypted file per server).
- Release signing and a designed icon.
