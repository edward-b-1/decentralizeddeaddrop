# deaddropd

The Decentralized Dead Drop server: one program and one SQLite file. Design:
`docs/PROTOCOL.md`.

## Running

```
deaddropd [serve] [OPTIONS]
deaddropd fingerprint [--db FILE] [--tls-cert FILE --tls-key FILE]
deaddropd operator-token [--db FILE] [--public HOST:PORT]

--db FILE               Database file                      [default: deaddrop.db]
--listen ADDRESS:PORT   Where to listen                    [default: 127.0.0.1:7443]
--public HOST:PORT      The address phones use, to print ready-made links
--tls-cert FILE         Certificate (PEM) from a certificate authority, with
--tls-key FILE          its private key (PEM)
--no-tls                Plain HTTP (development, or behind a local TLS proxy)
```

On first start the server prints:
- its **fingerprint**, which phones use to recognise it;
- a one-time **operator token**: the first device to register with it runs the server;
- with `--public`, a ready-made `deaddrop://` link holding both, and the same link as a QR
  code drawn in the terminal, to scan with the app.

`operator-token` issues a new token (for a lost operator phone, or a second operator);
with `--public` it prints the link and QR code too.
`fingerprint` prints the fingerprint again.

By default the server listens on this computer only. To be reachable, give it an address
such as `--listen 0.0.0.0:7443` and open that port in the firewall.

## TLS and the fingerprint

Every connection is TLS 1.3. Design: `docs/PROTOCOL.md` section 2.

- With no options, the server makes its own key (`deaddrop.key.pem`) and self-signed
  certificate (`deaddrop.cert.pem`) beside the database on first start. No hostname or
  certificate authority is needed.
- The **fingerprint** is the SHA-256 of the server's public key, in base32. To check it
  independently:

  ```
  openssl x509 -in deaddrop.cert.pem -pubkey -noout | openssl pkey -pubin -outform der | sha256sum
  ```

  (that prints the same hash in hexadecimal).
- **Back up the key file and keep it private.** It is the server's identity. If it is
  lost, phones can no longer recognise the server and every member must add it again
  from a new link. The server never replaces a missing key on its own; a missing
  certificate is simply remade from the key.
- With `--tls-cert` and `--tls-key` the server uses your own certificate and key, for
  example from Let's Encrypt. Phones still go by the fingerprint, which stays the same
  across renewals as long as the key is kept.
- `crates/tls` holds the fingerprint code and the check a phone makes
  (`pinned_client_config`).

## Logs

One line per request: method, route *pattern*, status, milliseconds. Never a device ID,
channel ID, name, address, token or query string. Successful queue traffic is not logged
at all. Database errors are logged as the words "database error" in release builds.

```
GET /v1/channels/{channel} 401 0ms
```

### The one exception: wrong server passwords

A wrong server password is logged with the time (UTC) and the address it came from, so a
firewall tool can ban whoever is guessing:

```
2026-11-26T18:20:05Z password-failure from=203.0.113.7
2026-11-26T18:20:09Z password-refused from=203.0.113.7
```

- `password-failure`: the password was wrong.
- `password-refused`: the attempt was turned away unchecked, because that address had
  already sent ten wrong passwords in ten minutes.

Nothing else is ever logged with an address. Someone who knows the password does not
appear, nor does anyone using an invite or any other way in.

Example fail2ban files are in `deploy/fail2ban/`. `docs/DEPLOYMENT.md` covers running the server on a machine of your own.

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
| DELETE | `/v1/devices/me` | device | Delete this device, its memberships and the channels it owns. Optional `leaves`: a signed leave for each channel it is in, kept for those channels' owners |

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

A new server accepts invite tokens only.

**Wrong server passwords** are limited to ten in ten minutes per network address (per /64
network for IPv6). After that the address is refused the password way in until ten
minutes after its last wrong guess. This is the only use the server makes of an address:
the count is held in memory only, each wrong guess is logged (see Logs), and a caller who
gets the password right is never recorded. If a proxy is put in front of the server, every
caller appears to come from the proxy, and the limit becomes one shared limit.

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
| GET | `/v1/channels/{channel}/members` | member | Signed roster, who has left since (`left`), what the leavers signed (`leaves`), every member's keys |
| DELETE | `/v1/channels/{channel}/members/me` | member | Leave. `leave`: the member's signed leave (optional, for versions up to 0.1.1) |

A new roster may list anyone in the roster it replaces and anyone who has asked to join,
and nobody else. Delivery starts only for those who asked: a member who has left stays
left for as long as the owner goes on listing them.

`left` is the server's own word and nothing more. A **leave** is a record the leaving
device signs (`deaddrop/leave/v1`: channel, device ID, the roster version it held), and
is what a channel owner's phone checks before it signs a roster without that member. The
server checks a leave before keeping it, keeps it with the signing key (the device may
be deleted by the time anyone reads it), and deletes it when the owner's roster no
longer lists the member or lets them back in.

### Messaging keys and prekeys

| Method | Path | Who | Purpose |
|---|---|---|---|
| PUT | `/v1/keys` | device | Publish signed messaging `keys`; add `one_time` prekeys (up to 50 a time, 100 kept) and a `fallback` |
| GET | `/v1/keys` | device | How many of my prekeys are left |
| POST | `/v1/channels/{channel}/keys/claim` | member | `devices` → each one's signed messaging keys and one prekey |

Messaging keys are a signed record (context `deaddrop/messaging-keys/v1`) signed by the
device's own signing key. A prekey is `{"key", "signature"}`, signed with the Ed25519 key
in that record over `deaddrop/prekey/v1\n<device>\n<key>`. A one-time prekey is handed
out once and deleted; when none are left the fallback is handed out instead. `GET
/v1/channels/{channel}/members` also returns each member's `messaging_keys`.

The server checks that messaging keys are signed by the device publishing them. Phones
do not rely on that and check the whole chain themselves.

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
