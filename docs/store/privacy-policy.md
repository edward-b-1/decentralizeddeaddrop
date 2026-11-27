# Privacy policy (draft)

*Draft text for `decentralizeddeaddrop.com/privacy`. Not published yet. The parts in
[square brackets] are for you to fill in.*

**Decentralized Dead Drop** is a messaging app made by [name, as an individual developer]
("we"). Last updated: [date].

## The short version

We receive nothing. The app has no account with us, no connection to any server of
ours, no analytics, no adverts and no crash reporting. It talks only to the servers you
add to it, which are run by you or by people you know, not by us.

## What the app keeps on your phone

- Your identity keys, in the phone's secure hardware. They never leave it.
- For each server you add: its address and fingerprint, your channels, their members'
  names, your message history, and the everyday keys that seal and open messages. These
  are stored encrypted, protected by a key held in the phone's secure hardware, and are
  left out of phone backups.
- Two settings: whether to stay connected in the background, and whether notifications
  show message text.

Removing a server in the app destroys your keys and history for it. Uninstalling the
app destroys all of it.

## What goes to a server you add

A server is run by whoever gave you its address, and they are responsible for it. By
design it receives as little as possible:

- **It can see:** a device ID that the app makes for that server alone (a different one
  for every server, and not a hardware identifier); the names of the channels you are
  in; the name you chose in each channel; who else is in them; when messages pass
  through and how large they are, to the nearest size step; and, as any server on the
  internet does, the network address you connect from.
- **It cannot see:** what your messages say. They are encrypted on your phone so that
  only the members of the channel can open them.
- **It keeps messages** only until every member has received them, or until the maximum
  age the channel's owner has set, and then wipes them.
- **Its log** records no addresses and no identifiers, with one exception: an address
  that gets the server's password wrong.

Taking a server off your phone (Servers → the server → Remove from this phone) deletes
your device, your place in its channels and the channels you own from that server.

## The camera and notifications

The camera is used only to scan QR codes, when you ask it to. Pictures are read on the
phone and are neither kept nor sent anywhere. Notifications are made on the phone
itself: the app uses no push service, so nothing about your messages passes through
Google or anyone else.

## Children

The app is not intended for anyone under 18.

## Changes and contact

If this policy changes, the new version will be published here with a new date.
There is no support address. Questions can be raised as an issue at
<https://github.com/edward-b-1/decentralizeddeaddrop/issues>, which is public: put nothing private in one.
