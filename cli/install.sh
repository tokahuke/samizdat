#! /usr/bin/env bash

# A quick and dirty script to build and install the samizdat CLI

if [ "$(uname)" == "Darwin" ]; then
    cargo build --release && sudo cp ../target/debug/samizdat /usr/local/bin
elif [ "$(expr substr $(uname -s) 1 5)" == "Linux" ]; then
    cargo build --relase && cp ../target/debug/samizdat $HOME/.local/bin
fi
