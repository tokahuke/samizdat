#! /usr/bin/env bash

printenv
envsubst '$SAMIZDAT_PUBLIC_KEY,$VERSION' < install.sh
