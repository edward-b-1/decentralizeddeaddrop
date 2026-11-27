# To do

What is left, roughly in the order it matters. Decisions already made are at the end so
that they are not reopened by accident. Started 27 November 2026.

## In hand

- [x] **Icon.** Chosen 27 November 2026: "sealed conversation"
      (`docs/design/icon-concepts-blue-grey-2026-11-27/`). Redrawn as vector shapes and built
      into the app (home screen, themed and status bar versions) and the website's pages.
      Seen on the emulator. Released in version 0.1.1. The store pictures are still to
      make (see Google Play).
- [x] **Message database.** Built for version 0.1.1 (27 November 2026): history is one
      database encrypted as a whole (SQLCipher), read 200 messages at a time, with the old
      per-server file taken in on first start. Tried on the emulator, including the
      upgrade from 0.1.0. Released in version 0.1.1.

## Trying it for real

Version 0.1.0 has run on one real phone, as the operator of a real server (27 November
2026). Version 0.1.1 (the icon and the encrypted message database) has run on the
emulator only. Still untried:

- [ ] Two real phones in one channel: messages both ways, join requests, safety numbers
      compared by scanning.
- [ ] The app closed overnight on mobile data: do messages still arrive, and what does
      it cost in battery? Battery settings on Samsung and Xiaomi phones.
- [ ] Scanning the QR code a server prints in a terminal, with a real camera. (It has
      only been read from screenshots.) Terminals with odd line spacing may draw a code
      a camera cannot read.
- [ ] A phone restart with the app never opened afterwards.
- [ ] A real fail2ban ban. The filter matches the server's lines, but the only address
      tried was on the ignore list.

## The app

- [ ] **Invite links that can be tapped.** A `deaddrop://` link is not made tappable by
      most messengers, so a friend far away has to copy and paste it. Idea: send the
      invite as an `https://decentralizeddeaddrop.com/...` link that opens the app, with
      the secret after the `#` (which browsers never send). The page needs a small
      script, and the site has none today.
- [ ] Changing a server's address on the phone (hostname to IP address, or after a
      move). Today the server has to be added again.
- [ ] Reading a QR code from a picture, for an invite sent as a screenshot.
- [ ] Disappearing messages: a timer per channel. The message database now allows it.
- [ ] A lock on the app (fingerprint or PIN).
- [ ] Pictures, audio and video (PROTOCOL.md section 11).

## The server and deployment

- [ ] `deaddropd operator-token` as root on a systemd install writes to a database owned
      by the service's own user. Tried only for reading (`fingerprint`).
- [ ] The website advertises HTTP/3 but the firewall only opens TCP 443. Harmless
      (browsers fall back); open UDP 443 or stop advertising it.
- [ ] A second way to ship the server than "build it yourself": a published container
      image or a ready-made binary.

## Google Play

Not started. `docs/PLAY_STORE.md` has the detail.

- [ ] A contact email address. Play requires one on every listing; there is none today.
- [ ] Decide whether Google is given the existing signing key. If not, a phone cannot
      move between the website's version and Play's without uninstalling. The choice is
      made once, when the app is created in the Play Console.
- [ ] Blocking and reporting. Play's policy for apps where people exchange content
      expects both, and this app will not have them (see Decisions). Expect this to be
      raised in review.
- [ ] Icon, feature graphic and screenshots.
- [ ] Twelve testers for fourteen days.

## Before telling anyone to rely on it

- [ ] An independent review of the design and the code. The encryption library has been
      audited; this project has not.
- [ ] A licences check each release (`python tools/licences.py`).
- [ ] Back up the signing key and the server's key file somewhere off the machines that
      hold them.

## Decisions (not to do)

- **No blocking and no reporting** (27 November 2026). Servers are private and run by
  the people who use them; a member can leave a channel and an owner can remove a member.
- **No delivery reports and no read receipts** (26 November 2026).
- **No support email address** (27 November 2026). The website points to GitHub issues.
- **The app is called "Dead Drop" on the phone** (27 November 2026), so that the name
  fits under an icon. The full name is used everywhere else.
- **Taking a server off a phone deletes the channels that phone owns there** (27
  November 2026).
- **Licence: PolyForm Noncommercial 1.0.0** (27 November 2026). "Source available", not
  "open source".
