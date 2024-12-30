#! /usr/bin/env bash


set -e

cargo zigbuild --release                \
    --target x86_64-pc-windows-gnu      \
    --package samizdat                  \
    --package samizdat-node
