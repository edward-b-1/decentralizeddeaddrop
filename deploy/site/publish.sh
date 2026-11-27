#!/bin/sh
# Publishes the website: copies site/ to the web server set up by setup-site.sh.
#
#     sh deploy/site/publish.sh USER@HOST
#
# Files already on the server that are not in site/ (the app downloads in download/)
# are left alone.
set -e
[ -n "$1" ] || { echo "Usage: sh deploy/site/publish.sh USER@HOST" >&2; exit 1; }
cd "$(dirname "$0")/../.."

tar -C site --exclude=README.md -cf - . |
    ssh "$1" 'tar -C /srv/decentralizeddeaddrop -xf - --no-same-owner && chmod -R a+rX /srv/decentralizeddeaddrop'
echo "Published to $1."
