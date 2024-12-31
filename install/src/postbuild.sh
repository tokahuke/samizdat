#! /usr/bin/env bash

set -e

cd ../get-samizdat
git stash
git checkout main
git pull
mkdir -p dist && cp -r ../dist/* dist
git add .
git diff-index --quiet HEAD || # prevents empty commit (that's an error)
    git commit -m "build of version $VERSION on `date +'%Y-%m-%d %H:%M:%S%z'`"
git push
