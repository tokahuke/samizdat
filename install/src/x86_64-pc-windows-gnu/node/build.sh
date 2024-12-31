#! /usr/bin/env bash

set -e

echo "Starting to build MSI installer"
makensis installer.nsi
