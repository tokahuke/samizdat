#! /usr/bin/env bash
#
# Cut a new Samizdat release.
#
# Usage:  ./release.sh <new-version>      # e.g. ./release.sh 0.2.0
#         ./release.sh --dry-run <new-version>
#
# What it does:
#   1. Sanity-checks the working tree (must be on `stable`, clean,
#      up to date with origin/stable).
#   2. Bumps `workspace.package.version` in the root `Cargo.toml`.
#      All workspace crates inherit via `version.workspace = true`,
#      so a single edit covers common/node/hub/cli/proxy/service.
#   3. Refreshes `Cargo.lock` (cargo update -w).
#   4. Commits + tags `v<version>`.
#   5. Pushes the branch + the tag.
#   6. Triggers the `build-artifacts.yaml` workflow against the new
#      tag via `gh workflow run`.
#
# After this script returns, watch the action in GitHub. The action
# pushes binaries into the `get-samizdat` submodule and that's the
# user-visible publish.

set -euo pipefail

DRY=0
if [[ "${1:-}" == "--dry-run" ]]; then
    DRY=1
    shift
fi

NEW_VERSION="${1:-}"
if [[ -z "$NEW_VERSION" ]]; then
    echo "usage: $0 [--dry-run] <new-version>" >&2
    exit 2
fi

# Loose semver check; not bulletproof, just catches typos.
if ! [[ "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.-]+)?$ ]]; then
    echo "release.sh: '$NEW_VERSION' does not look like semver (x.y.z)." >&2
    exit 2
fi

run() {
    if [[ $DRY -eq 1 ]]; then
        echo "+ $*"
    else
        echo "+ $*"
        "$@"
    fi
}

# --- preflight ---------------------------------------------------------------

BRANCH="$(git rev-parse --abbrev-ref HEAD)"
EXPECTED_BRANCH="${RELEASE_BRANCH:-stable}"
if [[ "$BRANCH" != "$EXPECTED_BRANCH" ]]; then
    echo "release.sh: must run on '$EXPECTED_BRANCH', currently on '$BRANCH'." >&2
    echo "  (override with RELEASE_BRANCH=... if you know what you're doing.)" >&2
    exit 1
fi

if ! git diff --quiet || ! git diff --cached --quiet; then
    echo "release.sh: working tree has uncommitted changes." >&2
    git status --short >&2
    exit 1
fi

git fetch origin "$EXPECTED_BRANCH"
LOCAL="$(git rev-parse HEAD)"
REMOTE="$(git rev-parse "origin/$EXPECTED_BRANCH")"
if [[ "$LOCAL" != "$REMOTE" ]]; then
    echo "release.sh: local '$EXPECTED_BRANCH' is not in sync with 'origin/$EXPECTED_BRANCH'." >&2
    echo "  local:  $LOCAL" >&2
    echo "  remote: $REMOTE" >&2
    exit 1
fi

CURRENT_VERSION="$(grep -E '^version\s*=' Cargo.toml | head -1 | sed -E 's/^version *= *"([^"]+)".*/\1/')"
if [[ "$CURRENT_VERSION" == "$NEW_VERSION" ]]; then
    echo "release.sh: workspace is already at $NEW_VERSION; pick a higher version." >&2
    exit 1
fi

if git rev-parse -q --verify "refs/tags/v$NEW_VERSION" >/dev/null; then
    echo "release.sh: tag 'v$NEW_VERSION' already exists." >&2
    exit 1
fi

echo "release.sh: $CURRENT_VERSION -> $NEW_VERSION on '$EXPECTED_BRANCH'."
echo

# --- bump --------------------------------------------------------------------

# Match only the workspace.package version line under [workspace.package].
# We rely on it being the second `^version = "..."` line in Cargo.toml
# (the first is irrelevant; in our layout there is no `[package]`
# section in the root manifest). sed -i differs between BSD and GNU, so
# use a portable in-place pattern.
TMP="$(mktemp)"
awk -v new="$NEW_VERSION" '
    /^\[workspace\.package\]/ { in_wp = 1 }
    in_wp && /^version[[:space:]]*=/ {
        sub(/"[^"]+"/, "\"" new "\"")
        in_wp = 0
    }
    { print }
' Cargo.toml > "$TMP"
if [[ $DRY -eq 1 ]]; then
    echo "+ would rewrite Cargo.toml workspace.package.version -> $NEW_VERSION"
    diff -u Cargo.toml "$TMP" || true
    rm -f "$TMP"
else
    mv "$TMP" Cargo.toml
fi

run cargo update -w

# --- commit + tag + push -----------------------------------------------------

run git add Cargo.toml Cargo.lock
run git commit -m "release v$NEW_VERSION"
run git tag -a "v$NEW_VERSION" -m "v$NEW_VERSION"
run git push origin "$EXPECTED_BRANCH"
run git push origin "v$NEW_VERSION"

# --- kick off the build ------------------------------------------------------

if command -v gh >/dev/null 2>&1; then
    run gh workflow run build-artifacts.yaml --ref "v$NEW_VERSION"
    echo
    echo "release.sh: workflow dispatched. Track with:"
    echo "  gh run watch"
else
    echo "release.sh: 'gh' not on PATH; dispatch the workflow manually:" >&2
    echo "  https://github.com/<org>/samizdat/actions/workflows/build-artifacts.yaml" >&2
fi
