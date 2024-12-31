#! /usr/bin/env bash


set -e


cargo zigbuild --release                \
    --target aarch64-apple-darwin       \
    --package samizdat                  \
    --package samizdat-node

cargo zigbuild --release                \
    --target x86_64-pc-windows-gnu      \
    --package samizdat                  \
    --package samizdat-node

cargo zigbuild --release                \
    --target x86_64-unknown-linux-gnu   \
    --package samizdat                  \
    --package samizdat-node             \
    --package samizdat-hub              \
    --package samizdat-proxy
