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

## Before it goes live

- [ ] A build of the app to offer: put it at `download/deaddrop-<version>.apk` on the
      server (not in this repository).
- [ ] `download.html`: the version, the size, the file name, the file's SHA-256 and the
      signing certificate's SHA-256 (the comment marked RELEASE shows where). Every
      release changes all but the last.
- [ ] `privacy.html`: the date, who makes the app, and a contact address that works.
- [ ] Every statement on the site is still true of the code. Public claims must match
      what is built.

To get the two fingerprints for a release:

```
sha256sum deaddrop-<version>.apk
apksigner verify --print-certs deaddrop-<version>.apk
```
