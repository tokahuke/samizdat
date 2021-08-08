#! /usr/bin/env bash

sudo systemctl disable --now samizdat-node
sudo rm /usr/local/bin/samizdat-node
sudo rm -rf /var/samizdat/node
sudo rmdir /var/samizdat || "samizdat folder not empty"
