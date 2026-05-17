#! /usr/bin/env bash
#
# Pull the latest set of binaries from the local samizdat-node's view
# of the `get-samizdat` collection, atomically replace
# /usr/local/bin/samizdat-{hub,node,proxy,}, and restart the services.
# This is how the testbed updates itself after first bootstrap: from
# the samizdat network, via its own subscription -- no out-of-band
# fetch.
#
# Triggered by .github/workflows/update-testbed.yaml. Safe to run
# standalone over SSH.

set -euo pipefail

# The proxy's target_node is the local samizdat-node on 4510. Fetching
# from the same hostname/port the proxy would serve, but without going
# through the TLS terminator, keeps this script independent of the
# proxy being healthy.
NODE_LOCAL="http://localhost:4510"
COLLECTION_PATH="/~get-samizdat/latest/x86_64-unknown-linux-gnu"

STAGE="$(mktemp -d /tmp/samizdat-update.XXXXXX)"
trap 'rm -rf "$STAGE"' EXIT

fetch() {
    local role="$1" name="$2"
    local url="$NODE_LOCAL$COLLECTION_PATH/$role/$name"
    echo "fetch: $url"
    curl -fsSL --max-time 60 -o "$STAGE/$name" "$url"
    # Reject empty / HTML-error responses; the proxy might 200 with an
    # error page if the collection drifted under us.
    if [[ ! -s "$STAGE/$name" ]]; then
        echo "fetch failed: $url returned empty body" >&2
        exit 1
    fi
}

fetch hub   samizdat-hub
fetch node  samizdat-node
fetch node  samizdat
fetch proxy samizdat-proxy

# Atomic replace (install(1) writes to a temp file in the destination
# directory then renames into place, so an in-flight execve sees
# either the old or the new binary, never a partial one).
install -m 755 "$STAGE/samizdat-hub"   /usr/local/bin/samizdat-hub
install -m 755 "$STAGE/samizdat-node"  /usr/local/bin/samizdat-node
install -m 755 "$STAGE/samizdat"       /usr/local/bin/samizdat
install -m 755 "$STAGE/samizdat-proxy" /usr/local/bin/samizdat-proxy

# Restart node first so the downstream proxy reconnects to a fresh
# version. The hub is independent of node/proxy on the request path,
# so its order is just "after the others to keep the network reachable
# for a beat longer."
systemctl restart samizdat-node
systemctl restart samizdat-proxy
systemctl restart samizdat-hub

echo "self-update complete."
