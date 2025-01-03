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

# Set preifx and a temporary work directory:
urlprefix=http://proxy.hubfederation.com/~get-samizdat/$VERSION/x86_64-unknown-linux-gnu/node
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
