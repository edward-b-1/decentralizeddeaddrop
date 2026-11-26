# deaddrop-client

The client engine: everything an app does apart from drawing screens and holding the
device's hardware keys. The Android app will call this code; `deaddrop` is a command-line
client built on the same code, for testing servers and for scripts.

## The command-line client

```
cargo run -p deaddrop-client -- --state FILE <command>
```

Each state file is one device on one server. **It holds that device's keys unprotected**
(a phone keeps them in secure hardware), so treat it like a password.

A whole conversation, with a server started as `deaddropd --public 127.0.0.1:7443`:

```
deaddrop --state anna.json join "deaddrop://127.0.0.1:7443/#fp=...&t=..."   # the link the server printed
deaddrop --state anna.json server-invite                                    # prints a link for Ben
deaddrop --state ben.json  join "<that link>"

deaddrop --state anna.json new-channel Family --as Anna --max-age 86400
deaddrop --state anna.json invite Family                                    # prints a channel invite
deaddrop --state ben.json  ask "<that link>" --as Ben
deaddrop --state anna.json requests Family                                  # shows Ben's device ID
deaddrop --state anna.json admit Family <device ID>

deaddrop --state ben.json  receive
deaddrop --state anna.json send Family hello Ben
deaddrop --state ben.json  listen                                           # prints messages as they arrive
```

`deaddrop --help` lists every command.

## The engine

`Account` is one identity on one server (`src/account.rs`). It is plain blocking code.

- The device's two long-term keys are reached only through the `DeviceKeys` trait, so on a
  phone they stay in the Android Keystore.
- Every method that changes key state takes a `save` callback and calls it at the moments
  the state must be stored: after a message is sealed and before it is posted, and after
  messages are opened and before the server is told it may wipe them.
- Everything a server says is checked: its key against the fingerprint, a channel's owner
  against the invite, the roster against the owner's signature, each member's keys
  against the roster, and versions against what was seen before.

`tests/engine.rs` runs the engine against a real server over real TLS.
