#! /usr/bin/env bash
#
# Sync the freshly-built artifacts into the `get-samizdat` distribution
# repo and push a release commit. Runs from `install/dist/` (set by the
# builder), so `../get-samizdat` resolves to `install/get-samizdat/`
# inside this workspace -- which is a git submodule pointing at the
# public release repo.
#
# `pipefail` so a failing rsync/cp doesn't yield a misleading commit.

set -euo pipefail

: "${VERSION:?VERSION must be set by the build env (env/version.sh).}"

RELEASE_BRANCH="${RELEASE_BRANCH:-main}"

cd ../get-samizdat

# Refuse to overwrite a dirty submodule tree. The CI build context is
# clean by construction; a local run on a tree with WIP would silently
# bury those changes if we `git stash`'d them away.
if ! git diff --quiet || ! git diff --cached --quiet; then
    echo "postbuild: get-samizdat submodule has uncommitted changes; aborting." >&2
    git status --short >&2
    exit 1
fi

git checkout "$RELEASE_BRANCH"
git pull --ff-only

mkdir -p dist
cp -r ../dist/* dist
git add .

# `diff-index` returns non-zero when there's something to commit. Skip
# the commit step on no-op builds so we don't push an empty release.
if ! git diff-index --quiet HEAD; then
    git commit -m "build of version $VERSION on $(date +'%Y-%m-%d %H:%M:%S%z')"
    git push
else
    echo "postbuild: no artifact changes, nothing to commit."
fi
