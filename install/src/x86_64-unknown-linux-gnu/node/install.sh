#! /usr/bin/env bash


# This script installs the samizdat node and CLI on your local machine. For license and 
# copyright, see http://github.com/tokahuke/samizdat.
#
# You will need `sudo` to run this code.


set -e

# Only works on linux:
if [ "$(expr substr $(uname -s) 1 5)" != "Linux" ]; then
    echo "Not Linux"
    exit 1
fi

# Drop the uninstaller on disk before anything else so that if this
# install fails halfway through the user still has a way to clean up.
# Same heredoc lives in node/hub/proxy install.sh; the script itself is
# role-agnostic and skips whatever roles are not present.
mkdir -p /usr/local/sbin
cat > /usr/local/sbin/samizdat-uninstall <<'SAMIZDAT_UNINSTALL_EOF'
#! /usr/bin/env bash
# samizdat-uninstall -- remove Samizdat from this box.
#
#   sudo samizdat-uninstall            stop services + remove binaries.
#                                      /etc/samizdat and /var/lib/samizdat
#                                      are preserved (configs, series
#                                      private keys, bookmarks, cached
#                                      objects).
#   sudo samizdat-uninstall --purge    everything above PLUS wipe
#                                      /etc/samizdat and /var/lib/samizdat.
#                                      Series private keys are gone
#                                      permanently. The script also
#                                      removes itself.

set -e

PURGE=0
case "${1:-}" in
    "")       ;;
    --purge)  PURGE=1 ;;
    *)        echo "usage: $0 [--purge]" >&2; exit 2 ;;
esac

if [ "$(id -u)" -ne 0 ]; then
    echo "samizdat-uninstall must be run as root (e.g. via sudo)" >&2
    exit 1
fi

for role in node hub proxy; do
    unit="samizdat-$role"
    if systemctl list-unit-files "$unit.service" --no-legend 2>/dev/null \
            | grep -q "$unit.service"; then
        echo "stopping + disabling $unit"
        systemctl stop "$unit" 2>/dev/null || true
        systemctl disable "$unit" 2>/dev/null || true
        rm -f "/etc/systemd/system/$unit.service"
    fi
    rm -f "/usr/local/bin/$unit"
done

# The CLI rides along with the node role; remove it regardless.
rm -f /usr/local/bin/samizdat

systemctl daemon-reload

if [ "$PURGE" -eq 1 ]; then
    echo "purging /etc/samizdat and /var/lib/samizdat..."
    rm -rf /etc/samizdat
    rm -rf /var/lib/samizdat
    echo "samizdat purged."
    rm -f "$0"
else
    echo "samizdat uninstalled."
    echo "/etc/samizdat and /var/lib/samizdat preserved."
    echo "to wipe those too, re-run with --purge:"
    echo "  sudo $0 --purge"
fi
SAMIZDAT_UNINSTALL_EOF
chmod 755 /usr/local/sbin/samizdat-uninstall

# Set preifx and a temporary work directory:
urlprefix=https://proxy.hubfederation.com/~get-samizdat/$VERSION/x86_64-unknown-linux-gnu/node
tmpdir=/tmp/samizdat-install-$RANDOM
mkdir -p $tmpdir && cd $tmpdir

# Download artifacts:
curl $urlprefix/samizdat > samizdat
curl $urlprefix/samizdat-node > samizdat-node
curl $urlprefix/samizdat-node.service > samizdat-node.service
curl $urlprefix/node.toml > node.toml

# Mark executables:
chmod +x samizdat
chmod +x samizdat-node

# Move artifacts to their correct places:
cp samizdat-node /usr/local/bin
cp samizdat /usr/local/bin
cp samizdat-node.service /etc/systemd/system
mkdir -p /etc/samizdat && cp --no-clobber node.toml /etc/samizdat

# Enable service:
systemctl stop samizdat-node || echo 'No running node detected'
systemctl daemon-reload
systemctl enable --now samizdat-node

# Post install:
sleep 2
samizdat hub new testbed.hubfederation.com 'UseBoth'

# Remove temporary directory:
rm -rf $tmpdir
