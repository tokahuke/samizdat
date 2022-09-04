#! /usr/bin/env bash

###
#
# Uninstallation script for an installation from source (see `install.sh`).
#
###

if [ "$(uname)" == "Darwin" ]; then
    echo "Unimplemented!" && exit 1
elif [ "$(expr substr $(uname -s) 1 5)" == "Linux" ]; then    
    sudo systemctl disable --now samizdat-node
    sudo rm /usr/local/bin/samizdat-node
    sudo rm -rf /var/samizdat/node
    sudo rmdir /var/samizdat || "samizdat folder not empty"
fi
