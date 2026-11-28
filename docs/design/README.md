# Design mockups

Mockups agreed before building. Each `.html` file is a page on its own: open it in a browser.

| Mockup | File | Link |
|---|---|---|
| App mockup A: channels, a channel, channel details, maximum message age, add a server, check the server, a server you run. 2026-11-26 | `app-2026-11-26-a.html` | https://claude.ai/artifact/5fyaCFfBfKQB17z6dH2U1b |
| Website mockup A: home, download, run a server, how it works, privacy (five pages in one file; use the menu). 2026-11-27 | `site-2026-11-27-a.html` | https://claude.ai/artifact/4sqjRFCCN4u5ZYLZ1KdnvJ |
| Website mockup B: a bolder landing page holding only the core points, with the detail moved to "How it works". 2026-11-27 | `site-2026-11-27-b.html` | https://claude.ai/artifact/JpdXhNwUdTBvPHE3WzDg89 |
| Send status mockup A: how your own messages show whether the server has taken them (waiting to send, sent, not sent). 2026-11-28 | `send-status-2026-11-28-a.html` | https://claude.ai/artifact/GKmecNu83dRZZ6cUngVr84 |

App mockup A is the design for the first version, except that there is **no delivery status
under messages** (screen 2 shows "Delivered to everyone" and "Waiting for Gran and
Alex"). Decided 2026-11-26: no delivery reports and no read receipts.

**Send status mockup A is the chosen design** (decided 2026-11-28) and is built. It is
not a delivery report: it says whether your own server has taken a message, and nothing
about who has collected it. The app differs from screen 4 in one way (decided
2026-11-28): pressing or holding a message that is waiting or was not sent opens a box
that also offers to try it ("Try now", "Try again"), and a waiting message can be
deleted so that it is never sent.

**The icon is "sealed conversation"** (chosen 2026-11-27), from the pictures in
`icon-concepts-blue-grey-2026-11-27/`. Its clean drawing, which the app and the website
use, is `sealed-conversation.svg` in that folder.

**Website mockup B is the chosen design** (decided 2026-11-27). In the mockups the pages
share one file and a small script switches between them; the real site is to be plain
pages with no script, no cookies and its fonts served from the site itself. Details with
a dashed underline are placeholders.
