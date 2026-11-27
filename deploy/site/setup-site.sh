#!/usr/bin/env bash
# One-time set-up of a fresh Ubuntu 24.04 (or later) server for decentralizeddeaddrop.com.
# Copy this file and the Caddyfile beside it to the server, then, as root:
#
#   bash setup-site.sh <your-username> "<your SSH public key>"
#
# The same hardening as deploy/setup-vps.sh: creates a sudo user with your SSH key,
# turns off password and root SSH logins, enables automatic security updates and keeps
# system logs for at most a week. Then: fail2ban for SSH, a firewall open only for SSH
# and the website, the Caddy web server, and the folder the site is served from.
# Afterwards publish the pages with deploy/site/publish.sh.
set -euo pipefail

USER_NAME="${1:?usage: setup-site.sh <username> \"<ssh public key>\"}"
SSH_KEY="${2:?usage: setup-site.sh <username> \"<ssh public key>\"}"
[ "$(id -u)" = 0 ] || { echo "Run this as root." >&2; exit 1; }
here="$(cd "$(dirname "$0")" && pwd)"
[ -f "$here/Caddyfile" ] || { echo "Put the Caddyfile next to this script." >&2; exit 1; }
# A mistyped key would lock everyone out once passwords are turned off.
echo "$SSH_KEY" | ssh-keygen -l -f - >/dev/null || { echo "That is not an SSH public key." >&2; exit 1; }

echo "== updates"
export DEBIAN_FRONTEND=noninteractive
apt-get update -q
apt-get upgrade -yq
apt-get install -yq ufw unattended-upgrades fail2ban caddy ca-certificates curl

echo "== user $USER_NAME"
if ! id "$USER_NAME" >/dev/null 2>&1; then
    adduser --disabled-password --gecos "" "$USER_NAME"
fi
usermod -aG sudo "$USER_NAME"
install -d -m 700 -o "$USER_NAME" -g "$USER_NAME" "/home/$USER_NAME/.ssh"
grep -qxF "$SSH_KEY" "/home/$USER_NAME/.ssh/authorized_keys" 2>/dev/null \
    || echo "$SSH_KEY" >> "/home/$USER_NAME/.ssh/authorized_keys"
chown "$USER_NAME:$USER_NAME" "/home/$USER_NAME/.ssh/authorized_keys"
chmod 600 "/home/$USER_NAME/.ssh/authorized_keys"
# sudo without a password (the account has none; the SSH key is the only way in).
echo "$USER_NAME ALL=(ALL) NOPASSWD:ALL" > "/etc/sudoers.d/90-$USER_NAME"
chmod 440 "/etc/sudoers.d/90-$USER_NAME"

echo "== SSH: keys only, no root login"
# The name sorts before the files a hosting company's image may add: the first value wins.
cat > /etc/ssh/sshd_config.d/10-hardening.conf <<'CONF'
PermitRootLogin no
PasswordAuthentication no
KbdInteractiveAuthentication no
CONF
sshd -t
systemctl reload ssh 2>/dev/null || true   # 24.04+: socket-activated; new logins read the config anyway

echo "== fail2ban for SSH"
systemctl enable --now fail2ban

echo "== firewall"
ufw default deny incoming
ufw default allow outgoing
ufw limit 22/tcp comment 'ssh'
ufw allow 80/tcp comment 'http'
ufw allow 443/tcp comment 'https'
ufw --force enable
# Privacy: don't log blocked connections (each line records a source address).
ufw logging off

echo "== log retention (privacy): at most 7 days"
mkdir -p /etc/systemd/journald.conf.d
printf "[Journal]\nMaxRetentionSec=7day\n" > /etc/systemd/journald.conf.d/90-privacy.conf
systemctl restart systemd-journald
if [ -f /etc/logrotate.d/rsyslog ]; then
    sed -i -E "s/^\s*weekly$/\tdaily/; s/^\s*rotate [0-9]+$/\trotate 7/" /etc/logrotate.d/rsyslog
fi

echo "== automatic security updates"
dpkg-reconfigure -f noninteractive unattended-upgrades

echo "== website"
# The pages belong to your user, so publishing needs no root.
install -d -o "$USER_NAME" -g "$USER_NAME" /srv/decentralizeddeaddrop /srv/decentralizeddeaddrop/download
install -m 644 "$here/Caddyfile" /etc/caddy/Caddyfile
caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile
systemctl enable caddy
systemctl restart caddy

echo
echo "Done. Before closing this session, check in a NEW terminal that this works:"
echo "    ssh $USER_NAME@<server address>"
echo "Then publish the pages: sh deploy/site/publish.sh $USER_NAME@<server address>"
echo "(Five failed logins from one address block that address for ten minutes.)"
