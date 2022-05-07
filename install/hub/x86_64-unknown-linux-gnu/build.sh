#! /usr/bin/env bash

echo "Starting to build hub installer for x86_64-unknown-linux-gnu"

envsubst '$SAMIZDAT_PUBLIC_KEY,$VERSION' < install.sh > $OUTPUT/install.sh &&
cp samizdat-hub.service $OUTPUT
