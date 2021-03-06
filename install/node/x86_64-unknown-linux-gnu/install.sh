#! /usr/bin/env bash

if [ "$(expr substr $(uname -s) 1 5)" != "Linux" ]; then
    echo "Not Linux"
    exit 1
fi

urlprefix=https://proxy.hubfederation.com/_series/$SAMIZDAT_PUBLIC_KEY/$VERSION/node/x86_64-unknown-linux-gnu
tmpdir=/tmp/samizdat-install-$RANDOM

mkdir -p $tmpdir &&
cd $tmpdir &&

curl $urlprefix/samizdat-node > samizdat-node &&
curl $urlprefix/samizdat > samizdat &&
curl $urlprefix/samizdat-node.service > samizdat-node.service &&

(systemctl stop samizdat-node || echo 'No running node detected') &&
cp samizdat-node /usr/local/bin &&
cp samizdat /usr/local/bin &&
cp samizdat-node.service /etc/systemd/system/samizdat-node.service &&
systemctl daemon-reload &&
systemctl enable --now samizdat-node &&

rm -rf $tmpdir
