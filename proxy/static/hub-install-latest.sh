#! /usr/bin/env bash

if [ $1 == "test" ]; then
    export BASE_URL=http://localhost:8080/_static
else
    export BASE_URL=https://proxy.hubfederation.com/_static
fi

echo Dowloading $BASE_URL/samizdat-hub &&
curl -Ls -X GET $BASE_URL/samizdat-hub > /tmp/samizdat-node &&
echo Dowloading $BASE_URL/samizdat-hub.service &&
curl -Ls -X GET $BASE_URL/samizdat-hub.service > /tmp/samizdat-node.service &&

echo Will need SUDO powers to configure Systemd &&
sudo systemctl disable --now samizdat-hub &&
sudo cp /tmp/release/samizdat-hub /usr/local/bin/samizdat-hub &&
sudo cp /tmp/samizdat-hub.service /etc/systemd/system/samizdat-hub.service &&
sudo systemctl enable --now samizdat-hub &&
echo Done
