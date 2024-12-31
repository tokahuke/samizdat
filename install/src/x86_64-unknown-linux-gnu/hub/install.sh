#! /usr/bin/env bash

if [ "$(expr substr $(uname -s) 1 5)" != "Linux" ]; then
    echo "Not Linux"
    exit 1
fi

urlprefix=http://proxy.hubfederation.com/get-samizdat/$VERSION/hub/x86_64-unknown-linux-gnu
tmpdir=/tmp/samizdat-install-$RANDOM

mkdir -p $tmpdir &&
cd $tmpdir &&

curl $urlprefix/samizdat-hub > samizdat-hub &&
curl $urlprefix/samizdat-hub.service > samizdat-hub.service &&

chmod +x samizdat-hub

cp samizdat-node /usr/local/bin &&
cp samizdat /usr/local/bin &&
cp samizdat-node.service /etc/systemd/system/samizdat-server.service &&
systemctl enable --now samizdat-hub &&

rm -rf $tmpdir
