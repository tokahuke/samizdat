#! /usr/bin/env bash

if [ $1 == "test" ]; then
    export BASE_URL=http://localhost:8080/_static
else
    export BASE_URL=https://proxy.hubfederation.com/_static
fi

echo Dowloading $BASE_URL/samizdat-node &&
curl -Ls -X GET $BASE_URL/samizdat-node > /tmp/samizdat-node &&
echo Dowloading $BASE_URL/samizdat-node.service &&
curl -Ls -X GET $BASE_URL/samizdat-node.service > /tmp/samizdat-node.service &&

echo Will need SUDO powers to configure Systemd &&
sudo systemctl disable --now samizdat-node &&
sudo cp tmp/samizdat-node /usr/local/bin/samizdat-node &&
sudo cp tmp/samizdat-node.service /etc/systemd/system/samizdat-node.service &&
sudo systemctl enable --now samizdat-node &&
echo Done
