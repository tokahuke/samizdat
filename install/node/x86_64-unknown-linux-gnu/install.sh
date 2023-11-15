#! /usr/bin/env bash

set -e

if [ "$(expr substr $(uname -s) 1 5)" != "Linux" ]; then
    echo "Not Linux"
    exit 1
fi

urlprefix=https://proxy.hubfederation.com/get-samizdat/$VERSION/node/x86_64-unknown-linux-gnu
tmpdir=/tmp/samizdat-install-$RANDOM

mkdir -p $tmpdir
cd $tmpdir

curl $urlprefix/samizdat-node > samizdat-node
curl $urlprefix/samizdat > samizdat
curl $urlprefix/samizdat-node.service > samizdat-node.service

chmod +x samizdat-node
chmod +x samizdat

(systemctl stop samizdat-node || echo 'No running node detected')
cp samizdat-node /usr/local/bin
cp samizdat /usr/local/bin
cp samizdat-node.service /etc/systemd/system/samizdat-node.service
systemctl daemon-reload
systemctl enable --now samizdat-node
sleep 2

# Post install:
samizdat hub new testbed.hubfederation.com 'UseBoth'

rm -rf $tmpdir
