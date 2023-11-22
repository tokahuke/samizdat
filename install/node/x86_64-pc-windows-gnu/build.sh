#! /usr/bin/env bash

set -e

echo "Starting to build Windows service"
cd samizdat-service
cargo build --release --target=x86_64-pc-windows-gnu
cp ../../../../target/x86_64-pc-windows-gnu/release/samizdat-service.exe ../$OUTPUT
cd ..

echo "Starting to build MSI installer"
mkdir -p dist
cp $OUTPUT/* dist/
makensis installer.nsi
cp dist/samizdat-installer.exe $OUTPUT/