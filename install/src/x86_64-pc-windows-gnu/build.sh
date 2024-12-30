#! /usr/bin/env bash


set -e

cargo build --all --release --target x86_64-unknown-linux-gnu
