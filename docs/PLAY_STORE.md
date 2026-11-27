# Play Store submission

What is needed to get Decentralized Dead Drop into a Play Store test, and later into
production. Steps marked 👤 are yours, in the Play Console or your accounts. The answers
below were drafted from what the app does as of 27 November 2026 (version 0.1.0).
Re-check them whenever the app changes.

> The Play Console's forms change regularly. If a question below doesn't match what you
> see, go by the Console's own help text, and tell Claude so the answers can be updated.

## 1. Before the first upload

| | Item | Status |
|---|---|---|
| ✅ | Release build set up: shrunk, and signed when `android/keystore.properties` exists | `./gradlew bundleRelease` (after `sh android/build-engine.sh`) |
| 👤 | **Back up the signing key** (§6) | Created 27 November 2026. Back it up |
| 👤 | A contact address | Decided 27 November 2026: there is none, and the website points to GitHub issues. Google Play requires a contact email on every listing, so one is needed before the app goes there |
| ✅ | Website at `decentralizeddeaddrop.com` with the privacy policy | Live since 27 November 2026 (`site/`) |
| ⬜ | App icon, feature graphic, screenshots | Not made. The icon in the app is a placeholder |
| ⬜ | A way to block someone and to report a channel (§7) | Not built; Play expects it of apps where people exchange content |
| ⬜ | Tried on real phones | Only the emulator so far |
| 👤 | Recruit **12+ testers** (Android, Google account) for the closed test | To do |

## 2. Create the app 👤

Play Console → **Create app**:
- Package name: **`com.decentralizeddeaddrop`** (permanent; matches `applicationId` in `android/app/build.gradle.kts`)
- App name: **Decentralized Dead Drop**
- Default language: English (United Kingdom)
- App or game: **App**
- Free or paid: **Free**

In **App integrity**, keep **Play App Signing** on (the default). Google then holds the
key that signs what people install, and ours is only the *upload* key.

Invite links use the app's own `deaddrop://` scheme and never a website, so there is no
`assetlinks.json` to set up.

## 3. Store listing

**App name** (30 max): `Decentralized Dead Drop`

**Short description** (80 max):
> Private messaging on a server you or someone you trust runs. No account.

**Full description:**
> Decentralized Dead Drop is a private messenger with no company in the middle. There is no central service: you add a server that you, or someone you know, runs, and talk to the people on it.
>
> PRIVATE BY DESIGN
> • No account: no phone number, no email address, no password. Your phone's keys are your identity, and you get a fresh one for each server.
> • End-to-end encrypted. Messages are sealed on your phone and only the members of a channel can open them. The server cannot read them.
> • Keys move on with every message, so a key taken later cannot open what was said before.
> • Messages wait on the server only until everyone has received them, or until the maximum age the channel's owner has set, and are then wiped.
> • No delivery reports and no read receipts: nobody is told when you collected a message.
> • No adverts, no analytics, no tracking, and no push service: nothing about your messages passes through Google.
>
> YOUR SERVER, YOUR RULES
> • Run a server on any small Linux machine. It needs no domain name: phones recognise it by its fingerprint.
> • Choose how people get in: invites, a server password, approval of each new device, or open to anyone.
> • Channels are group chats with one owner, who alone lets people in and removes them. The server cannot add anyone.
> • Add as many servers as you like. Each sees a different identity, so they cannot tell that the same phone uses both.
>
> MADE TO BE CHECKED
> • Invite people with a QR code or a link.
> • Compare safety numbers to be sure you are talking to the right person.
> • The source code of the app and the server is public, so anyone can check it.
>
> Decentralized Dead Drop needs an internet connection and a server to connect to. This is an early version and has had no independent security review.

**Graphics:** to be made (512 × 512 icon, 1024 × 500 feature graphic, at least four phone screenshots).

**Category:** Communication. **Website:** `https://decentralizeddeaddrop.com`. **Privacy policy:** `https://decentralizeddeaddrop.com/privacy` (once the site exists).

## 4. App content (Policy → App content) 👤

### Ads
**No.**

### App access
**All functionality is available without special access**, but a reviewer needs a server.
Give them a link to a test server you run, for example:
> No login. Open this link on the test phone to add our test server: `deaddrop://<address>:7443/#fp=<fingerprint>&t=<token>` (or: Servers → Add a server → paste it). Then Channels → New channel. To exchange messages, a second phone joins with an invite from the channel's details.

Make the server **open to anyone** for the review, or give several invite links: each works once.

### Content rating (IARC questionnaire)
- **Category:** Communication.
- **Violence, sexual content, language, drugs, gambling:** none.
- **Users can interact or exchange content:** **Yes.**
- **Shares the user's current location:** No.
- **Digital purchases:** No.

### Target audience
**18+ only** is the simplest, and suits the app. It is not designed for children.

### Data safety
The app talks only to servers its user adds. **You, the developer, receive nothing**: there is no developer server, no analytics and no crash reporting. Play still counts data that leaves the phone, so declare what goes to the server a user chooses:

- **Is all of the user data encrypted in transit?** Yes (TLS 1.3; messages are also end-to-end encrypted).
- **Can users request that their data is deleted?** Yes: Servers → the server → Remove from this phone deletes the device, its memberships and the channels it owns from that server.

| Data type (Play's name) | What it is here | Collected / shared | Purpose | Optional? |
|---|---|---|---|---|
| Personal info → **Name** | The name someone chooses in each channel (any nickname) | Collected, not shared | App functionality | Required to be in a channel |
| Device or other IDs | The app's own device ID on each server (a hash of a key the app makes; not a hardware ID) | Collected, not shared | App functionality | Required |
| App activity → **Other user-generated content** | Channel names | Collected, not shared | App functionality | Required to create a channel |

**Not collected:** messages (end-to-end encrypted; the server cannot read them, and Play's guidance exempts data the recipient of the transmission cannot access. Confirm against the Console's help text for "Messages"). Contacts, photos, location, phone number, email address, diagnostics.

If the Console will not accept "collected" for data that goes to servers you do not run, describe it in the free-text field as: sent only to the server the user has added, which is run by the user or someone they know.

### Permissions and foreground services
Declare the **foreground service** type (Policy → App content → Foreground service permissions):
- **Special use** (`FOREGROUND_SERVICE_SPECIAL_USE`):
  > Decentralized Dead Drop is a messenger that uses no push service: nothing about a user's messages passes through Google or any third party. To receive messages while the app is closed it keeps a connection open to the servers the user has added (servers run by the user or people they know). A notification shows the whole time it is connected, and Settings → Stay connected turns it off. No other foreground service type describes this.
  - **Video:** show a server being added, the "Connected to …" notification, the app closed, a message sent from a second phone, and its notification arriving.

Other permissions:
- `CAMERA`: only to scan QR codes (invites, server links, safety numbers). Asked for when the scanner is first opened. Pictures are read on the phone and never kept or sent.
- `POST_NOTIFICATIONS`: new messages, and the notification that shows the app is connected.
- `RECEIVE_BOOT_COMPLETED`: to reconnect after the phone restarts.
- `INTERNET`.

### Other declarations
- **Government app:** No. **Financial features:** None. **Health:** No. **News app:** No.

## 5. Testing tracks 👤

1. **Internal testing** (up to 100 people, available within minutes): upload `android/app/build/outputs/bundle/release/app-release.aab`, add yourself, install from the Play link and check it works.
2. **Closed testing:** at least **12 testers**, opted in for **14 days in a row**. Ask them to use it: Google asks about this when you apply for production.
3. **Production:** apply after the 14 days, then use a staged rollout.

**Each upload needs a higher `versionCode`** (`android/app/build.gradle.kts`).

## 6. The upload key 👤 (important)

Created 27 November 2026: `C:\Users\Edward\.deaddrop\upload-keystore.jks` (outside the repository, as Pocket Radio's is). Its certificate's SHA-256 is
`E6:53:8A:A9:C7:2A:8B:C5:14:CF:BD:07:25:C7:18:6A:B7:C4:61:72:83:7D:B1:1D:CD:B5:17:97:D1:69:E5:69`, the "Signed by" value on the website's download page.

- **It already signs the app people download from the website.** If it is lost, nobody who installed from the website can ever be given an update: they would have to uninstall, losing their keys and messages.
- **Decide before creating the app in the Play Console:** by default Google signs the Play version with a key of its own, and then a phone cannot move between the website's version and Play's without uninstalling. To keep them interchangeable, choose to give Google this key when Play App Signing is set up (the option to use your own key). That choice cannot be changed afterwards.
- `android/keystore.properties` (git-ignored) tells the build where it is:
  ```
  storeFile=C:/Users/Edward/.deaddrop/upload-keystore.jks
  storePassword=...
  keyAlias=upload
  keyPassword=...
  ```
- **Back up both somewhere off this PC.**

## 7. Before production

- **Blocking and reporting.** Play's policy for apps where people exchange content expects a way to block a person and to report content. Today a member can leave a channel and an owner can remove a member; there is no "hide this person's messages" and no report. Because servers are run by their users, a report has nowhere central to go: the honest design is probably "hide this member on my phone" plus "tell the channel's owner / the person who runs the server". To be designed.
- Real-phone checks: scanning QR codes with a real camera, staying connected with the screen off on mobile data, battery settings on Samsung and Xiaomi phones, a restart.
- The website, with the privacy policy.
- An icon and the store graphics.
- An independent look at the cryptography would be worth having before telling anyone to rely on it.
