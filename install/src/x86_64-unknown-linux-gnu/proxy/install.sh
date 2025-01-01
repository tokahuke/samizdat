#! /usr/bin/env bash

set -e

# Only works on linux:
if [ "$(expr substr $(uname -s) 1 5)" != "Linux" ]; then
    echo "Not Linux"
    exit 1
fi

# Set preifx and a temporary work directory:
urlprefix=http://proxy.hubfederation.com/get-samizdat/$VERSION/x86_64-unknown-linux-gnu/proxy
tmpdir=/tmp/samizdat-install-$RANDOM
mkdir -p $tmpdir && cd $tmpdir

# Download artifacts:
curl $urlprefix/samizdat-proxy > samizdat-proxy
curl $urlprefix/samizdat-proxy.service > samizdat-proxy.service
curl $urlprefix/proxy.toml > nproxyode.toml

# Mark executables:
chmod +x samizdat-proxy

# Move artifacts to their correct places:
cp samizdat-proxy /usr/local/bin
cp samizdat-proxy.service /etc/systemd/system
mkdir -p /etc/samizdat && cp proxy.toml /etc/samizdat

# Enable service:
systemctl stop samizdat-proxy || echo 'No running proxy detected'
systemctl daemon-reload
systemctl enable --now samizdat-proxy

# Remove temporary directory:
rm -rf $tmpdir
