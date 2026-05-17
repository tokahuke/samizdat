#! /usr/bin/env bash
#
# Standalone helper: build only the Windows installer.exe, assuming the three
# `.exe` artifacts already exist at
# `target/x86_64-pc-windows-gnu/release/`. The full pipeline in
# `install/src/rust/build.sh` already runs this step inline; this script
# exists for local debugging when you only want to iterate on
# `installer.nsi`.

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../../../.." && pwd)"
TARGET_DIR="${REPO_ROOT}/target/x86_64-pc-windows-gnu/release"

if [ ! -f "${TARGET_DIR}/samizdat-node.exe" ]; then
    echo "Missing ${TARGET_DIR}/samizdat-node.exe; run the rust cross-build first."
    exit 1
fi

cd "${SCRIPT_DIR}"
mkdir -p dist
cp "${TARGET_DIR}/samizdat-node.exe"    dist/
cp "${TARGET_DIR}/samizdat-service.exe" dist/
cp "${TARGET_DIR}/samizdat.exe"         dist/

echo "Building NSIS installer..."
makensis installer.nsi

echo "Done: ${SCRIPT_DIR}/dist/samizdat-installer.exe"
