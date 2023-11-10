#! /usr/bin/env bash

# A quick and dirty script to build and install the samizdat CLI

if [ "$(uname)" == "Darwin" ]; then
    cargo build --release && sudo cp ./target/release/samizdat /usr/local/bin
elif [ "$(expr substr $(uname -s) 1 5)" == "Linux" ]; then
    cargo build --release && cp ../target/release/samizdat $HOME/.local/bin
fi
