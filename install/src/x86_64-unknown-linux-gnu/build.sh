#! /usr/bin/env bash


set -e

cargo zigbuild --release                \
    --target x86_64-unknown-linux-gnu   \
    --package samizdat                  \
    --package samizdat-node             \
    --package samizdat-hub              \
    --package samizdat-proxy
