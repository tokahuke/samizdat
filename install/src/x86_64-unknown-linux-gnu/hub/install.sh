#! /usr/bin/env bash

set -e

# Only works on linux:
if [ "$(expr substr $(uname -s) 1 5)" != "Linux" ]; then
    echo "Not Linux"
    exit 1
fi

# Set preifx and a temporary work directory:
urlprefix=http://proxy.hubfederation.com/get-samizdat/$VERSION/x86_64-unknown-linux-gnu/hub
tmpdir=/tmp/samizdat-install-$RANDOM
mkdir -p $tmpdir && cd $tmpdir

# Download artifacts:
curl $urlprefix/samizdat-hub > samizdat-hub
curl $urlprefix/samizdat-hub.service > samizdat-hub.service
curl $urlprefix/hub.toml > hub.toml

# Mark executables:
chmod +x samizdat-hub

# Move artifacts to their correct places:
cp samizdat-hub /usr/local/bin
cp samizdat /usr/local/bin
cp samizdat-hub.service /etc/systemd/system
mkdir -p /etc/samizdat && cp hub.toml /etc/samizdat

# Enable service:
systemctl stop samizdat-hub || echo 'No running hub detected'
systemctl daemon-reload
systemctl enable --now samizdat-hub

# Remove temporary directory:
rm -rf $tmpdir
