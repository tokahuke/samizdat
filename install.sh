#! /usr/bin/env bash

cargo build --release --bin samizdat-node &&
sudo systemctl disable --now samizdat-node
sudo cp target/release/samizdat-node /usr/local/bin/samizdat-node &&
sudo cp node/samizdat-node.service /etc/systemd/system/samizdat-node.service &&
sudo systemctl enable --now samizdat-node
