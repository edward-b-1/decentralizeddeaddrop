# Running your own Dead Drop server

A Dead Drop server is one small program, `deaddropd`, and one folder of data. It needs
no domain name, no certificate authority and no other service: it makes its own TLS key,
and phones recognise it by that key's fingerprint (PROTOCOL.md section 2).

Any always-on Linux machine with a public address will do. The smallest VPS a provider
sells is enough: the server holds messages only until they are delivered.

What you need to decide first:

- **The address phones will use.** The machine's public IP address, or a hostname you
  have pointed at it (an ordinary DNS A record; nothing else to set up). It goes into
  every link and QR code.
- **The port.** 7443 unless you have a reason to change it.

## The quick way: a fresh Ubuntu VPS and Docker

1. **Create the VPS** (Ubuntu 24.04 or later) and note its IP address.

2. **Lock it down.** Copy `deploy/setup-vps.sh` to the server and run it once as root:

   ```
   bash setup-vps.sh <your-username> "<your SSH public key>"
   ```

   It creates your user, turns off password and root logins over SSH, opens only SSH and
   port 7443 in the firewall, switches on automatic security updates, keeps system logs
   for at most a week, and installs Docker. Check you can log in as the new user from a
   second terminal before you close the first.

3. **Get the code and say where the server is.** As your new user:

   ```
   git clone https://github.com/edward-b-1/decentralizeddeaddrop.git
   cd decentralizeddeaddrop/deploy
   cp env.example .env
   nano .env                # set DEADDROP_PUBLIC to <the IP address or hostname>:7443
   ```

4. **Start it.** The first build takes a few minutes.

   ```
   docker compose up -d --build
   docker compose logs --no-log-prefix deaddropd
   ```

   The log of the first start shows the server's **fingerprint**, a one-time **operator
   token**, and a **link and QR code** holding both.

5. **Become the operator.** In the app: Servers, Add a server, Scan a QR code, and point
   the phone at the QR code in the terminal. (Or send yourself the link and open it on
   the phone.) That phone now runs the server: it switches the ways in on and off, makes
   invites and approves devices. The token works once.

   If the log has scrolled away or the token was never used, make a new one:

   ```
   docker compose exec deaddropd /deaddropd operator-token --db /data/deaddrop.db --public <address>:7443
   ```

6. **Let people in.** From the operator's phone: Servers, your server, Invite a device.
   Each invite lets one new phone on, once.

## Without Docker

`deaddropd` is a single program with no dependencies beyond the C library.

```
cargo build --release -p deaddropd            # on the server, or any machine of the same kind
sudo install -m 755 target/release/deaddropd /usr/local/bin/deaddropd
echo 'DEADDROP_PUBLIC=<address>:7443' | sudo tee /etc/deaddrop.env
sudo install -m 644 deploy/deaddropd.service /etc/systemd/system/deaddropd.service
sudo systemctl enable --now deaddropd
sudo journalctl -u deaddropd                  # the first start prints the link and QR code
```

The unit file runs the server as its own user with access to nothing but
`/var/lib/deaddrop`. Open the port as well (`sudo ufw allow 7443/tcp`). Commands that
read the data folder need root here, for example
`sudo deaddropd fingerprint --db /var/lib/deaddrop/deaddrop.db`.

## What to back up, and what happens if you lose it

Everything is in one folder: the Docker volume `deaddrop-data`, or `/var/lib/deaddrop`.

| File | What it is | If it is lost |
|---|---|---|
| `deaddrop.key.pem` | The server's TLS key. **Phones recognise the server by it.** | Every phone refuses the server. Everyone has to add it again as a new server. |
| `deaddrop.cert.pem` | The certificate that goes with the key | Made again from the key at the next start |
| `deaddrop.db` (and `-wal`, `-shm`) | Who is registered, channels and member lists, messages not yet delivered | Everyone has to register and be invited again |

Back up the key file once, somewhere safe: it never changes. Anyone who has it can
pretend to be your server, though they still could not read a single message.

To print the fingerprint at any time:

```
docker compose exec deaddropd /deaddropd fingerprint --db /data/deaddrop.db
```

## Updating

```
cd decentralizeddeaddrop && git pull
cd deploy && docker compose up -d --build
```

The data folder is kept, so phones notice nothing but a few seconds' reconnection.

## What the server knows and logs

It cannot read messages, and it cannot add anyone to a channel (PROTOCOL.md section 8).
It does know which devices are registered, which channels exist and what they are
called, who is in them, and when messages pass through.

Its log has one line for each request, with no address and no identifier in it. The one
exception is a wrong **server password**, which is logged with the address it came from
so that a firewall tool can act on it:

```
2026-11-26T18:20:05Z password-failure from=203.0.113.7
```

The server already refuses an address after ten wrong passwords in ten minutes.
`deploy/fail2ban/` has files for blocking such an address at the firewall as well.
fail2ban reads the lines and matches them; an address on fail2ban's own ignore list
(often the administrator's) is, as always, never banned.

If you put the server behind Docker Desktop, a load balancer or anything else that hides
callers' addresses, every caller looks the same to the password limit. On an ordinary
Linux VPS with the compose file here, addresses are seen as they are.

## Using a certificate from a certificate authority

There is no need to: the fingerprint is a stronger check than a certificate authority
gives. If you want one anyway (for instance to put other things on the same hostname),
start the server with `--tls-cert FILE --tls-key FILE`. The fingerprint then follows
that key, and changes whenever the key does, so tell your certificate tool to keep the
same key when it renews.
