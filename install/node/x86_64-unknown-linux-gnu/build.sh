#! /usr/bin/env bash

envsubst '$SAMIZDAT_PUBLIC_KEY' < install.sh > $OUTPUT/install.sh &&
cp samizdat-node.service $OUTPUT
