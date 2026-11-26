# Decentralized Dead Drop: protocol design

*Secure communication systems.*

**Status:** Design agreed 2026-11-26. The server and the message encryption are built
(§13); the app is not. Open decisions are listed in §12.

Decentralized Dead Drop ("Dead Drop" below) is a text messenger where every community runs its own server. The server is a
dead drop: it holds sealed messages only until they have been collected, and can never
read them. Audio, pictures and video come later (§11).

Much of the identity and membership design comes from Pocket Radio (`docs/NETWORKING.md`
in that repo). The message encryption does not; §6 explains why.

---

## 1. The pieces

| Piece | Job |
|---|---|
| **App** (Android) | Holds the keys, encrypts and decrypts, stores message history (encrypted on the phone). Can be connected to several servers at once. |
| **Server** (`deaddropd`) | One program plus one SQLite file. Registers devices, stores channel records and membership, queues sealed messages until delivered. Run by whoever wants one. |
| **Website** (`decentralizeddeaddrop.com`) | Information only. No user data ever goes to it. Apps never contact it. |

There is no central service of ours. Servers do not talk to each other. A person who is on
two servers has two unrelated identities (§4).

## 2. Adding a server

A server is added by **address + port + a way in** (§3). The address is an IP address or a
hostname, whichever the operator set up.

**Knowing it is the right server.** Every server has a TLS certificate. Two cases:
- *Hostname with a normal certificate* (e.g. Let's Encrypt): checked the usual way.
- *IP address only, or a self-made certificate:* the app checks the **fingerprint** of the
  server's key (SHA-256 of its public key) instead. The fingerprint travels in the invite
  link or QR code. Typed by hand, the app shows the fingerprint to compare on first
  connection and remembers it afterwards. A later change is a loud warning, never silent.

Invite link (also the QR content), handled by the app itself, never by a website:

```
deaddrop://HOST:PORT/#fp=<fingerprint>&auth=<method>&t=<token if any>
```

## 3. Ways in (server authentication)

The operator chooses which of these the server accepts. They gate **registering a device**;
after that the device proves itself by signing a challenge with its key (as Pocket Radio).

| Method | How it works | Good for |
|---|---|---|
| **Open** | Anyone who can reach the server may register. | Private networks, testing |
| **Server password** | One shared password (server keeps only an Argon2id hash). Wrong guesses are limited per network address. | Small trusted groups |
| **Invite token** | Operator creates tokens: single-use or limited-use, with an expiry. Carried in the invite link. | The normal case |
| **Approval** | The device registers as pending; the operator sees its ID and approves it. | Strict servers |

Methods can be combined (e.g. invite token *and* approval).

**The operator.** On first start the server prints a one-time **operator token**. The first
device to present it becomes the operator and manages the server from the app (tokens,
approvals, limits). The same actions are available from a command line on the server.

Being allowed onto a server grants nothing inside it. Channels have their own gate (§5).

### 3.1 Someone in the middle, or a fake server

Two separate layers protect two different things:

- **Message contents** are sealed on the sending phone and opened only on members' phones
  (§6). Nothing on the path, and not the server itself, can read or alter them. This holds
  even if the connection to the server is intercepted or the server is replaced.
- **The connection to the server** (TLS, checked by certificate or by fingerprint as in §2)
  protects what is *around* the messages: the way-in secret (password or token), device
  IDs, and which channels a device uses. It also stops a fake server from being accepted.

An attacker who takes over the server's IP address (through the network, the hosting
company, or because the operator gave the address up and someone else was assigned it)
does not have the server's private key, so the fingerprint check fails and the app refuses
to connect. A fingerprint is stricter than a normal certificate here: a certificate
authority will issue a certificate to whoever controls the address at that moment.

What remains, and how it is closed:
- **First contact with a hand-typed address** has no fingerprint to check against. The app
  shows it and asks for confirmation; invite links and QR codes avoid the problem.
- **A fake or hostile server handing out the wrong keys for a channel's members.** Channel
  invites carry the **owner's key fingerprint**, the member list must be signed by that
  owner (§5), and members can compare safety numbers (§4).
- **Blocking.** Anyone in the middle can stop messages getting through. Nothing prevents
  that; the app shows messages as undelivered.

## 4. Identity and keys

- No phone number, email or account password. **The device's keys are the account.**
- **A fresh identity per server**, so two servers cannot tell that the same phone uses both.
- Per identity, as in Pocket Radio: a **signing key** (ECDSA P-256) and an **agreement key**
  (ECDH P-256), both in the Android Keystore; the signing key vouches for the agreement key.
  Device ID = short hash of the signing public key.
- Display names are labels chosen per channel, not identities.
- **Safety numbers:** two people can compare a short code (or scan a QR) derived from both
  signing keys to confirm nobody is in the middle.

## 5. Channels

A channel is a group chat on one server. A two-person conversation is a channel with two
members; there is no separate one-to-one mechanism.

As in Pocket Radio:
- The creator is the **owner**. The owner's device signs the **channel record** (name,
  maximum message age §7, a version that only goes up).
- Joining needs an invite (link, code or QR) from the owner. The joiner then *asks* to join,
  and the owner lets them in. Nobody is put in a channel without asking.
- Only the owner adds or removes members.
- Lost owner phone = the channel can no longer change. Members can leave.

Changed from Pocket Radio: instead of one signed pass per member, the owner signs the
**whole member list** (the *roster*: device IDs and names, with a version that only goes
up). Every change of membership is a new roster. Phones never accept an older one.
- The server cannot add a member: it cannot sign a roster.
- The server cannot keep a removed member either. With separate passes it could have gone
  on presenting a removed member's old, still validly signed pass.
- A device ID is a hash of the device's signing key, so the roster also pins every
  member's key. Names are unique within a channel, whatever the capitals.
- When a member leaves, the server stops delivering to them at once and tells the others;
  the owner's next roster drops them.

## 6. Message encryption

### Why not Pocket Radio's scheme as it is

Pocket Radio gives a channel **one shared key per epoch**, wrapped for each member's device,
replaced only when someone is removed. That suits live voice. For messages it has a
weakness: anyone who copies the sealed traffic and later obtains that one key (from any
member's phone) can read **every message of that epoch**, past and future.

### The design: per-sender ratchets, handed over through private sessions

This is the design Signal uses for groups, and the one Matrix uses. It is implemented
with **vodozemac**, the Matrix project's library (why: §6.1), in `crates/crypto`.

1. **Messaging keys.** Besides its two hardware keys (§4), each device has a pair of
   everyday messaging keys (Curve25519 and Ed25519). The device **signs them with its
   hardware signing key** and publishes the signed record. So the chain of trust is:
   owner's signature on the roster → device ID → hardware signing key (the ID is its
   hash) → messaging keys. Every phone checks every link itself.
2. **Prekeys.** A device leaves a stock of signed, single-use public keys on the server
   so others can start a private session with it while it is offline, plus one
   "fallback" for when the stock has run out. The server hands each one-time prekey out
   once and deletes it.
3. **Private sessions** between two devices (the Double Ratchet; "Olm" in vodozemac).
   Keys move forward with every message and are deleted after use.
4. **Sending ratchets** (per-sender group ratchets; "Megolm"). Each device has its own
   per channel. Every message uses the next key and the used key is destroyed. A device
   hands the current position of its ratchet to each other member **through the private
   sessions**; those hand-overs are small messages in the same queue, addressed to one
   member.
5. **A message** is encrypted once with the sender's ratchet and signed with a key that
   belongs to that ratchet, so members cannot impersonate each other. The server stores
   one sealed copy however many members there are.
6. **A sending ratchet is replaced** whenever the roster changes, and otherwise every 100
   messages or 7 days. The new one is handed only to the members in the roster at that
   moment, so a removed member gets nothing, and a new member gets nothing older.
7. **Receiving ratchets are wound forward** past each message once it has been opened.
   Neither that message nor anything earlier from that sender can be opened again, even
   by someone who later takes the phone and its stored data.
8. **Padding:** messages are padded to fixed size steps (256 bytes, then doubling), so
   every short text looks the same size.

What this gives beyond Pocket Radio's scheme:
- **Forward secrecy:** a phone seized today cannot open messages captured earlier.
- **Limited damage from a leak:** a copied ratchet reads only until it is next replaced.

What stays the same: device keys as identity, owner-signed membership, the server never
holding anything it can open.

What a phone does with what the server tells it: nothing on trust. Which channel a
message is in and who sent it are the server's claims; a message only opens with keys
that the real sender handed over for that channel.

Limits to know about:
- **A member who has not yet fetched a new roster** keeps using their old sending ratchet
  until they do, and a removed member holds that ratchet. The server stops delivering to
  a removed member at once, so they would also need the server's help to read on.
- **A hand-over that expires from the queue** before its recipient collects it leaves
  that sender's following messages unreadable for that recipient until the sender's next
  ratchet. Asking the sender to hand over again is not built yet.
- **Messages from one sender must be opened in the order sent** (the queue delivers them
  in order). One that turns up after a later one has been opened cannot be opened.
- **Lost phone, lost history,** as before. And a device's own sent messages are kept by
  the app; they cannot be re-opened from the wire.

### 6.1 Why vodozemac (the trial, 2026-11-26)

| Option | Finding |
|---|---|
| **Write it ourselves** on standard primitives | Possible, but it would be new, unreviewed code doing the most delicate job in the system. |
| **Signal's library** (libsignal) | AGPL licence. Signal states that use outside Signal is unsupported and that its interfaces change without notice. It is not published as a Rust package, and it depends on Signal's own forks of other libraries. |
| **MLS** (RFC 9420, OpenMLS) | Audited (2026) and MIT licensed, but every member must process every change to the group, in order. A queue that deletes undelivered messages at a maximum age breaks that: a phone that was off for longer would fall out of the group. |
| **vodozemac** (chosen) | Apache 2.0, pure Rust, published, actively maintained. Exactly the design above. Independently audited (Least Authority, 2022) and used by Matrix clients. |

Known points about vodozemac, so nobody is surprised later:
- A researcher reported weaknesses in February 2026. The significant one (a missing check
  on key-exchange results) was fixed in version 0.10.0; this project uses 0.11.2.
- Private-session messages carry a shortened (64-bit) authentication tag in the one
  session version Matrix has specified. The full-length variant is still marked
  experimental upstream and is not used here. Channel messages are also signed.
- vodozemac's own password-style encryption of stored state is not used. Stored state is
  handed to the app as bytes, to be encrypted with a key the phone's secure hardware
  protects.
- The audit covers vodozemac, not this project. **Decentralized Dead Drop itself has had
  no independent review.**

## 7. The delivery queue

1. A device sends a sealed message for a channel.
2. The server stores **one sealed copy** and a **pending mark for each other member device**.
3. Each recipient device receives it over its open connection, saves it, and **acknowledges**.
   The server removes that device's pending mark.
4. **When no pending marks remain, the sealed copy is wiped at once** (SQLite secure delete,
   so the bytes are overwritten, not just unlinked).
5. **Maximum message age:** a message that has been in the queue for the channel's maximum
   message age is wiped, whoever has not yet collected it.

A message leaves the queue in exactly these two ways: delivered to all recipients, or
maximum message age reached.

**Maximum message age is a per-channel setting** (decided 2026-11-26). The channel owner
sets it; it is part of the signed channel record, so every member's phone can show it and
the server cannot quietly change it.

Consequences, by design:
- **The server has no history.** A new member, or a new phone, sees nothing from before.
- **A lost phone loses its messages.** No cloud backup.
- *Not built yet:* telling a sender which members a message expired for.
- "Wiped" means the row is overwritten with zeros in the database and the database's log
  is folded away at once; a test searches the files for the bytes. What the disk hardware
  keeps underneath (SSDs remap blocks) is outside the server's control, which is one more
  reason the server only ever holds sealed data.

The queue also carries the small protocol messages of §6 (sender-key shares), under the
same rules.

## 8. What the server can and cannot do

| A compromised or hostile server could… | …but not |
|---|---|
| Refuse service; drop or delay messages | Read any message (never holds a key) |
| See which device IDs are in which channel, and when they send | Forge a message from a member (signatures) |
| See message sizes (padded) and connection addresses | Add a member other phones accept, or keep a removed one (owner-signed roster) |
| Keep sealed copies it claims to have wiped | Open them later, even with a seized phone (forward secrecy) |

Logging follows Pocket Radio's rule: nothing that identifies a user or device is logged
(no IDs, addresses, tokens or channel names).

One exception, decided 2026-11-26: to stop someone guessing the server password, the
server counts **wrong** password attempts per network address for ten minutes, and logs
each one with the address so that a firewall tool can ban the guesser. Someone who knows
the password is never recorded, and neither is anyone using another way in.

## 9. Receiving messages in the background

Version 1: while switched on, the app keeps one connection open to each of its servers
(a foreground service, like Pocket Radio's radio service). Nothing goes through Google.
This costs some battery, more with many servers. A content-free wake-up through a push
service can be offered later as an opt-in.

## 10. On the phone

- Message history in an encrypted database; the database key is protected by the Keystore.
- Later: disappearing messages (a timer per channel), screen lock for the app.

## 11. Later: audio, pictures, video

- **Attachments:** encrypted on the phone with a one-off key, uploaded in chunks, the key
  sent inside a normal message. Same queue rules: wiped when every member has fetched it or
  the maximum message age passes. Operators set a size limit.
- **Live calls:** a separate project (as Pocket Radio did with LiveKit).

## 12. Open decisions

1. **Server language: Rust** (proposed). One Rust "core" library holds the whole protocol
   and is used by the server's test client *and* by the Android app, so there is one
   implementation instead of matching Kotlin and Go versions. Needs the Android NDK and
   Rust's Android targets installed on this PC.
2. ~~Encryption~~ **Decided 2026-11-26: §6 as proposed** (Signal's design: Pocket Radio's
   membership plus keys that roll forward with every message).
3. ~~Build it ourselves, Signal's library, or MLS?~~ **Decided 2026-11-26:** none of the
   three; vodozemac (§6.1).
4. ~~Queue age~~ **Decided 2026-11-26:** maximum message age is a per-channel setting (§7).
5. ~~Display name~~ **Decided 2026-11-26:** "Decentralized Dead Drop", tag line "Secure
   communication systems."

## 13. What is built (2026-11-26)

`crates/core` (shared formats) and `crates/server` (`deaddropd`), in Rust. See
`crates/server/README.md` for the API.

Built and tested: the four ways in and the operator (§3), sign-in by signed challenge,
channel records, invites, join requests and rosters (§5), messaging keys, prekeys and the
message encryption (§6, `crates/crypto`), the delivery queue with both ways out (§7), live
delivery over a WebSocket, and logging that holds no identifiers except for wrong server
passwords. End-to-end tests run whole clients through the server, including a server
that swaps keys and a removed member handed every sealed message.

Not built yet:
- **TLS and the server fingerprint** (§2). The server speaks plain HTTP and listens on
  this computer only by default. Nothing should be deployed until this is done.
- **The Android app**, and the bridge that lets it use `crates/core` and `crates/crypto`.
- **Asking again for a lost hand-over** (§6), and recovering a private session that has
  gone out of step.
- The **safety numbers** of §4 and everything in §9 to §11.
