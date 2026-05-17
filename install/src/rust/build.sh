#! /usr/bin/env bash

set -e


cargo zigbuild --release                \
    --target aarch64-apple-darwin       \
    --package samizdat                  \
    --package samizdat-node

cargo zigbuild --release                \
    --target x86_64-pc-windows-gnu      \
    --package samizdat                  \
    --package samizdat-node             \
    --package samizdat-service

cargo zigbuild --release                \
    --target x86_64-unknown-linux-gnu   \
    --package samizdat                  \
    --package samizdat-node             \
    --package samizdat-hub              \
    --package samizdat-proxy


# Homebrew distribution tarball.
(
    cd target/aarch64-apple-darwin/release
    tar -czvf samizdat.tar.gz samizdat samizdat-node
)

# Windows NSIS installer. We stage the three .exe artifacts into a `dist/`
# subdir next to `installer.nsi` so the `File "dist/..."` directives in the
# .nsi resolve correctly, then run makensis. The resulting
# `samizdat-installer.exe` is exported by build.yaml.
(
    cd install/src/x86_64-pc-windows-gnu/node
    mkdir -p dist
    cp ../../../../../target/x86_64-pc-windows-gnu/release/samizdat-node.exe    dist/
    cp ../../../../../target/x86_64-pc-windows-gnu/release/samizdat-service.exe dist/
    cp ../../../../../target/x86_64-pc-windows-gnu/release/samizdat.exe         dist/
    makensis "-DVERSION=${VERSION:-0.0.0}" installer.nsi
)
