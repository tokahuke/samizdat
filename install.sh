#! /usr/bin/env bash

if [ "$(uname)" == "Darwin" ]; then
    cargo build --release --bin samizdat-node &&
    sudo cp target/release/samizdat-node /usr/local/bin/samizdat-node &&
    sudo cp node/samizdat-node.plist /Library/LaunchDaemons &&
    sudo launchctl load /Library/LaunchDaemons/samizdat-node.plist
elif [ "$(expr substr $(uname -s) 1 5)" == "Linux" ]; then
    cargo build --release --bin samizdat-node &&
    sudo systemctl disable --now samizdat-node
    sudo cp target/release/samizdat-node /usr/local/bin/samizdat-node &&
    sudo cp node/samizdat-node.service /etc/systemd/system/samizdat-node.service &&
    sudo systemctl enable --now samizdat-node
fi
