#! /usr/bin/env bash

# A quick and dirty script to build and install the samizdat CLI

cargo build && cp ../target/debug/samizdat $HOME/.local/bin
