#!/usr/bin/env bash
# One-time setup of a fresh Ubuntu 24.04 (or later) server for Dead Drop. Run as root:
#
#   bash setup-vps.sh <your-username> "<your SSH public key>" [port]
#
# Creates a sudo user with your SSH key, turns off password and root SSH logins, opens
# only SSH and the Dead Drop port (7443 unless you give another), enables automatic
# security updates, keeps system logs for at most a week, and installs Docker.
set -euo pipefail

USER_NAME="${1:?usage: setup-vps.sh <username> \"<ssh public key>\" [port]}"
SSH_KEY="${2:?usage: setup-vps.sh <username> \"<ssh public key>\" [port]}"
PORT="${3:-7443}"

echo "== updates"
export DEBIAN_FRONTEND=noninteractive
apt-get update -q
apt-get upgrade -yq
apt-get install -yq ufw unattended-upgrades ca-certificates curl git

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
cat > /etc/ssh/sshd_config.d/10-deaddrop.conf <<'CONF'
PermitRootLogin no
PasswordAuthentication no
KbdInteractiveAuthentication no
CONF
systemctl reload ssh 2>/dev/null || true   # 24.04+: socket-activated; new logins read the config anyway

echo "== firewall"
ufw default deny incoming
ufw default allow outgoing
ufw allow 22/tcp            # SSH
ufw allow "$PORT/tcp"       # Dead Drop
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

echo "== Docker"
if ! command -v docker >/dev/null; then
    curl -fsSL https://get.docker.com | sh
fi
usermod -aG docker "$USER_NAME"

echo
echo "Done. Before closing this session, check in a NEW terminal that this works:"
echo "    ssh $USER_NAME@<server address>"
echo "Then carry on with docs/DEPLOYMENT.md, step 3."
