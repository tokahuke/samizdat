#! /usr/bin/env bash

if [ "$(uname)" == "Darwin" ]; then
    
elif [ "$(expr substr $(uname -s) 1 5)" == "Linux" ]; then    
    sudo systemctl disable --now samizdat-node
    sudo rm /usr/local/bin/samizdat-node
    sudo rm -rf /var/samizdat/node
    sudo rmdir /var/samizdat || "samizdat folder not empty"
fi
