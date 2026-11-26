# deaddropd

The Decentralized Dead Drop server: one program and one SQLite file. Design:
`docs/PROTOCOL.md`.

## Running

```
deaddropd [serve] [--db FILE] [--listen ADDRESS:PORT]     defaults: deaddrop.db, 127.0.0.1:8080
deaddropd operator-token [--db FILE]
```

On first start it prints a one-time **operator token**. The first device to register with
that token runs the server. `operator-token` issues a new one (for a lost operator phone,
or a second operator).

**It speaks plain HTTP for now** and by default listens on this computer only. TLS and the
server fingerprint (PROTOCOL.md §2) are not built; do not expose it to a network yet.

## Logs

One line per request: method, route *pattern*, status, milliseconds. Never a device ID,
channel ID, name, address, token or query string. Successful queue traffic is not logged
at all. Database errors are logged as the words "database error" in release builds.

```
GET /v1/channels/{channel} 401 0ms
```

## API (version 1)

JSON over HTTP. Everything except the first four routes needs
`Authorization: Bearer <session token>` from an approved device.

Keys and signatures are standard base64 of DER (P-256 SubjectPublicKeyInfo; ECDSA over
SHA-256). IDs and tokens are lowercase base32. A *signed record* is
`{"record": "<the JSON text that was signed>", "signature": "<base64>"}`; the signature
covers `<context>\n<record>` where the context is `deaddrop/channel/v1` or
`deaddrop/roster/v1` (`crates/core/src/records.rs`).

### Getting in

| Method | Path | Who | Purpose |
|---|---|---|---|
| GET | `/v1/server` | anyone | Server ID and which ways in are on |
| POST | `/v1/devices` | anyone | Register: `signing_key`, `agreement_key`, `agreement_sig`, and `token` or `password` if needed |
| POST | `/v1/sessions/challenge` | anyone | `device_id` → `challenge` |
| POST | `/v1/sessions` | anyone | `device_id`, `challenge`, `signature` over `deaddrop/session/v1\n<server_id>\n<challenge>` → session `token` (one hour) |
| GET | `/v1/devices/me` | device | Who am I |
| DELETE | `/v1/devices/me` | device | Delete this device, its memberships and the channels it owns |

A device ID is base32 of the first ten bytes of SHA-256 of the signing key. `agreement_sig`
is the signing key's signature over `deaddrop/agreement-key/v1\n<base64 agreement key>`.

### Operator

| Method | Path | Purpose |
|---|---|---|
| GET, PUT | `/v1/operator/settings` | `open`, `tokens`, `approval` |
| PUT | `/v1/operator/password` | `{"password": "..."}` sets it, `null` removes it |
| POST | `/v1/operator/invites` | `uses`, `ttl` (seconds) → `token` |
| GET | `/v1/operator/devices` | Every device and its status |
| POST | `/v1/operator/devices/{device}/approve` | Approve a pending device |
| DELETE | `/v1/operator/devices/{device}` | Remove a device from the server |

A new server accepts invite tokens only. Wrong server passwords are limited to ten a
minute across all callers, since the server keeps nothing that tells callers apart.

### Channels

| Method | Path | Who | Purpose |
|---|---|---|---|
| GET | `/v1/channels` | device | My channels and my open join requests |
| POST | `/v1/channels` | device | Create: signed `channel` record and a signed `roster` naming only the owner |
| GET | `/v1/channels/{channel}` | device | Signed record, owner's key, roster version |
| PUT | `/v1/channels/{channel}` | owner | New signed record (higher version) |
| DELETE | `/v1/channels/{channel}` | owner | Delete the channel and its queue |
| POST | `/v1/channels/{channel}/invites` | owner | `uses`, `ttl` → `token` |
| POST | `/v1/channels/{channel}/join-requests` | device | Ask to join: `token`, `name` |
| GET | `/v1/channels/{channel}/join-requests` | owner | Who is asking, with their keys |
| DELETE | `/v1/channels/{channel}/join-requests/{device}` | owner | Decline |
| PUT | `/v1/channels/{channel}/roster` | owner | New signed roster (higher version): the only way to add or remove members |
| GET | `/v1/channels/{channel}/members` | member | Signed roster, who has left since, every member's keys |
| DELETE | `/v1/channels/{channel}/members/me` | member | Leave |

### The queue

| Method | Path | Who | Purpose |
|---|---|---|---|
| POST | `/v1/channels/{channel}/messages` | member | `body` (base64, up to 64 KiB sealed), optional `to` (some members only) |
| GET | `/v1/queue` | device | Everything waiting for me, oldest first |
| POST | `/v1/queue/ack` | device | `ids`: I have safely stored these |
| GET | `/v1/events` | device | WebSocket: live delivery |

A message is deleted when every recipient has acknowledged it, or when it is older than
the channel's maximum message age, whichever comes first. A member who is removed, leaves
or deletes their device no longer counts as a recipient.

**WebSocket.** First frame from the client: `{"token": "<session token>"}`. The server
answers `{"type":"ready"}`, then sends `{"type":"message","id","channel","sender","body","sent","expires"}`
for everything waiting and everything new, and `{"type":"changed"}` when a channel, roster
or join request the device can see has changed. The client acknowledges with
`{"ack": ["<id>", ...]}`.

## Tests

`cargo test -p deaddropd`. `tests/on_disk.rs` runs against a real database file and
searches the files for a message's bytes after it has been delivered or has expired.
