# The website (decentralizeddeaddrop.com)

Information about the project and the app download. Plain pages: no script, no cookies,
no analytics, and nothing loaded from anyone else (the fonts are in `fonts/`). The app
never contacts this site.

Open `index.html` in a browser to look at it. The design it was built from is
`docs/design/site-2026-11-27-b.html`; these pages are now the source and are edited by hand.

| File | Page |
|---|---|
| `index.html` | Landing page |
| `download.html` | The app download (also served as `/download`) |
| `server.html` | Run a server |
| `how-it-works.html` | What protects messages and what does not |
| `privacy.html` | Privacy policy (the Play Store listing points at `/privacy`) |
| `style.css`, `fonts/`, `icon.svg` | Shared by every page |

## Serving it

`deploy/site/Caddyfile` is the web server configuration: HTTPS with an automatic
certificate, no visitor log, and headers that forbid scripts and anything from another
site. Copy this folder to `/srv/decentralizeddeaddrop` on the server. The site must be
served that way for its own claims ("keeps no visitor log") to be true.

- `deploy/site/setup-site.sh`: one-time set-up and hardening of a fresh server (run
  there, as root, with the Caddyfile beside it).
- `sh deploy/site/publish.sh USER@HOST`: copies this folder to the server.

## Releasing a version of the app

The site went live on 27 November 2026 with version 0.1.0.

1. `python tools/licences.py`, if any dependency changed. It rewrites the licence notices
   in the app and `licences.txt` here.
2. Raise `versionCode` and `versionName` in `android/app/build.gradle.kts` (and the
   version shown in the app's Settings).
3. `sh android/build-engine.sh`, then `./gradlew assembleRelease` in `android/`. The
   build is signed when `android/keystore.properties` exists (`docs/PLAY_STORE.md`
   section 6). Every version must be signed with the same key, or phones refuse the
   update.
4. Try the built file on a phone or the emulator.
5. In `download.html` (the comment marked RELEASE shows where): the version, the size,
   the file name and the file's SHA-256. The "Signed by" fingerprint never changes.

   ```
   sha256sum app-release.apk
   apksigner verify --print-certs app-release.apk
   ```
6. Copy the file to the server as `/srv/decentralizeddeaddrop/download/deaddrop-<version>.apk`,
   then `sh deploy/site/publish.sh USER@HOST`.

Also check before publishing: every statement on the site is still true of the code.
