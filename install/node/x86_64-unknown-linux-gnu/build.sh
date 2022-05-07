#! /usr/bin/env bash

envsubst '$SAMIZDAT_PUBLIC_KEY,$VERSION' < install.sh > $OUTPUT/install.sh &&
cp samizdat-node.service $OUTPUT
