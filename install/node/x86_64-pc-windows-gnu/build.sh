#! /usr/bin/env bash

set -e

echo "Starting to build Windows service"
cd samizdat-service
cargo build --release --target=x86_64-pc-windows-gnu
cp ../../../../target/x86_64-pc-windows-gnu/release/samizdat-service.exe ../$OUTPUT
cd ..
