# Deferred work

Single list of everything the audits surfaced but did not fix. Each
entry says **what** it is, **why it was deferred**, and **where to
start** when someone picks it up.

Per-pass narrative (which were fixed, the why-not-fixed reasoning, and
confirmed-not-bugs) lives in [`audit-history.md`](audit-history.md);
this file is the actionable backlog only.

> Re-reading entries before re-flagging: many of these look scary in
> isolation and have non-obvious reasons for being deferred. Read
> `audit-history.md`'s "Confirmed not bugs" sections before claiming a
> deferred item is actually fixable in a one-liner.

## Priority order (core protocol + hub)

1. **Per-IP connection cap in hub QUIC accept.** The only item from
   the original audit's hub queue that survives the three-axes
   pass as a real today-bug. A flood from one IP fills the global
   `max_connections` pool pre-throttle; the connection-level
   defenses do not get a chance to apply. Default cap of 64
   trades NAT tolerance against the worst case (a botnet of 32
   distinct IPs still fills the 2048 pool). Hygiene, not full DoS
   defense.

(The original priority queue was eight items. Walking each through
"is there an attacker who can do something not already bounded?"
collapsed seven of them to no-ops -- their threats are already
closed by content hashing, the Riddle scheme, the per-connection
throttle, or the hub admin's loopback-only binding. The antibodies
below preserve the trace so a future audit pass does not refile
them.)

## Subscription eager-fetch is silent

`Edition::refresh` in `node/src/models/series.rs` spawns
`hubs().query_with_retry(...)` for each item in a new edition's
inventory and discards the `JoinHandle` (`.map(|_| ())`). When the
publisher node goes offline between announcement and the eager fetch
landing (e.g. a `publish-get-samizdat` CI run that exits the moment
its `Wait for testbed to mirror` step passes), some objects never
arrive at the subscriber. No log line marks the per-item failure, so
the partial mirror is invisible until a client tries to fetch and the
on-demand query also fails (no peer has the bytes either).

Fix: log per-item eager-fetch outcomes (`Some(_)` / `None`) so partial
mirrors are visible in `journalctl -u samizdat-node`. Small.

Vacuum protection is already handled: `SeriesRef::advance`
Reference-bookmarks every inventory item of the new edition at
advance time (`series.rs:414-426`, labelled "B4: Bookmark
accounting"). Earlier framing of this entry claimed subscription
content was non-bookmarked -- wrong. The `BookmarkType::Subscription`
suggestion was overengineering a problem that doesn't exist.

Related but separate gap, surfaced while re-reading: dropping a
subscription does not release the Reference bookmarks placed by past
advances. Without a subsequent advance, the bookmarks stay forever.
Not a bug today (subscriptions are rarely dropped) but the pinner
control plane will need a "drop subscription + walk last edition +
unbookmark" admin action; see `docs/roadmap.md`.

Surfaced on 2026-06-08 while debugging intermittent 404s on
`series-v5bk....hubfederation.com/latest/install.sh` after a
publish-get-samizdat run.

## Antibodies: things that look like bugs and aren't

Each entry below summarises an audit-flagged item that does not
survive a careful trace. Left in place so a future audit pass
does not re-discover the apparent severity, file the same
urgent-sounding entry, and spend a cycle on a real-but-bounded
hygiene issue dressed up as a correctness or security bug.

### Hub reconnect "silently drops queries on the surviving direction"

The `future::select` on the two reset receivers in
`node/src/system/mod.rs::HubConnectionInner::connect` (and the
mirror in `hub/src/rpc/hub_as_node.rs::connect`) was flagged as
silently dropping queries. Walking the code: receivers always
resolve -- the asker gets either the value (if the old server
task delivers before its transport dies) or `RecvError::Closed`
when the last sender on the old `candidate_channels` map is
dropped, and `query_with_retry` retries on backoff. The old
server task lingers as an orphan holding the old QUIC
connection but self-collects when its transport errors or
quinn's idle timeout fires. "Full QUIC re-handshake on every
tarpc wobble" is a real but unobserved cost. Resource hygiene
at worst, not a correctness bug.

### Cryptographic `ChannelId` binding

The audit flagged that `recv_candidate` on both
`hub/src/rpc/hub_server.rs` and `hub/src/rpc/hub_as_node.rs`
accepts an unauthenticated channel id any connected peer can
inject on. Sounds serious. But the per-connection
`call_throttle` + `call_semaphore` already bound each
connection's injection rate. HMAC-binding would not change that
bound -- a malicious peer is throttle-limited whether they
inject on a `cc` issued to them specifically or on a `cc`
shared across broadcast targets. Same rate, same window.
Content hashing closes the wrong-content axis; Riddle closes
the privacy axis. Do NOT spend a wire-format break on this.

### Hub admin token middleware (`/blacklisted-ips` unauth)

The hub admin HTTP binds loopback only. The realistic attacker
is a process on the hub host itself. On the testbed that means
"anyone with SSH access" -- already game over. If you ever run
a multi-tenant hub host, revisit; until then, no work to do.

### Replay-resistance with signed timestamps

Current protection is per-nonce dedup in a 10-minute window
plus the per-connection throttle. A replayed message past the
dedup window costs the hub one unit of throttle-bounded work.
Messages cannot carry forged content (riddles + signatures).
Signed timestamps would be pure defense-in-depth on top of an
already-bounded attack.

### `Riddle::riddle_for` padding

"Leaks message length, e.g. IPv4 vs IPv6." The hub processes
the candidate addresses to forward them; it already knows. The
only attacker who could learn IPv4-vs-IPv6 from message length
but otherwise not know it is an on-wire eavesdropper -- and
QUIC encrypts message bytes, while packet-size traffic
analysis carries many other signals that padding the riddle
does not address. No concrete attacker who benefits.

### `Matcher` cleanup-task bound

Self-tagged "perf, not correctness." At max load (12 q/s/node
x 2048 connections) tokio handles thousands of cleanup-task
spawns per second. `DelayQueue` would be a real improvement
but the current shape is not a bottleneck. Refile when you see
it in a profile, not before.

### Per-entity scoping of `ManageSeries`

The OAuth-style consent UI is the *intended* boundary; a
finer-grained scope set shifts the decision into a longer
consent screen rather than into the API.
`docs/threat-model.md` explicitly declares "local multi-tenant
browser usage is not a supported configuration today." By
design, not a bug.

### Hub HTTP body size cap

The entry self-flagged: "Only matters if the proxy is ever
pointed at a remote node; loopback-only deployments do not
need it." Today's only deployment is loopback-only.

### `Message<T>.validation` field at `common/src/riddles.rs:13-18`

The struct carries a `validation: u16` initialised to zero. A
fresh-eyes audit will spot it, conclude it does nothing, and
propose deletion. AES-GCM-SIV's AEAD already authenticates the
payload; the constant-zero byte adds nothing. *But* deletion is
a wire-format break (bincode is positional) for zero functional
gain. Same shape as the H9 channel-id-binding antibody. Leave
it in place across protocol breaks; only consider removing if a
larger riddle redesign forces a wire bump anyway.

### `ChannelId::random` / `Hash::rand` panic on `getrandom` failure

`common/src/address.rs:36-40` and `common/src/hash.rs:96-100`
both `.expect("getrandom failed")`. An audit will recommend
returning `Result` for graceful degradation. But a failed
`getrandom(2)` means the kernel CSPRNG is dead, in which case
every other crypto primitive in the process is also unsafe to
run. The panic is the correct shape; propagating would force
unreachable error handling at every call site.

### `node_server::LAST_REFRESH_ATTEMPT` GC polarity

`node/src/system/node_server.rs:28-48`. An audit will claim
"GC only fires on the accepted path; rejected attempts leak
entries forever." Read the polarity: the map is only *inserted
into* on the accepted path (line 41). Rejected attempts take
the early `return false` at line 38 without writing. The GC
covers every insertion path; there is no leak.

### `IdentityProvider::set_endpoint` "race" with in-flight reads

`node/src/identity_dapp/mod.rs:96-145`. The struct holds a
`tokio::sync::RwLock<Contract<...>>`. An audit will worry that
`set_endpoint` (writer) and a concurrent `get()` (reader) race
on observability of the swap. `tokio::sync::RwLock` already
guarantees the writer awaits all readers; the swap is atomic
from any reader's perspective. Platform guarantee, not a bug.

### `Statistics` lock ordering vs LMDB writes

`hub/src/rpc/node_sampler.rs:158-177`. An audit will claim
"RwLock writer is held across a blocking LMDB write," citing
the `self.log()` call near `acquire-then-write`. Read the
bracket order: `start_request` calls `self.log()` *before* the
lock; `end_request_with_success` calls `drop(lock)` *before*
`self.log()`. The LMDB write never runs under the lock. The
comment at line 159 says so explicitly; trust it.

### Route53 `wait_for_insync` "swallowing" timeout

`proxy/src/dns/route53.rs:213-243`. The function logs a warn
and returns on timeout instead of propagating an error. An
audit will see this as a swallowed failure. The docstring
declares it intentional: ACME's own outer poll loop catches
the case where DNS never converges. Returning `Err` here
would just bubble through to an outer retry that does the
same thing. Cooperative timeout coordination, not a bug.

## Under-audited areas (known unknowns)

The second pass mapped the node-hub lifecycle, the protocol, and
the file-sharing algorithm. The hub-to-hub federation path was
touched only tangentially. Likely sources of undiscovered bugs:

- **Multi-hop candidate routing.** Every hop adds a `channel_id`
  indirection in its `candidate_channels` map. Cleanup paths when
  an intermediate hub disconnects, or when the asker times out
  partway through the chain, are not well exercised.
- **Deadline propagation across hops.** Whether `Context.deadline`
  flows correctly through `Hub::resolve` ->
  `HubAsNodeServer::resolve` -> next-hop `resolve`, or gets reset to
  `context::current()` at a hop, is not verified. If reset,
  malicious hubs can extend deadlines. If propagated, deep-hop
  responses may be silently discarded.
- **Replay-resistance under cycle.** `ReplayResistance::check` is
  keyed by the message nonce; cycles in the federation graph ought
  to be cut by it because the nonce is preserved across hops. Worth
  a property test.
- **`HubAsNodeServer::recv_candidate` has no throttle.** The
  client-facing path goes through `throttle`; the partner side does
  not. A malicious partner can blast it faster than a normal node
  could.
- **Forwarding-task lifecycle on partner disconnect.** When a
  partner hub mid-chain drops, the forwarding tokio task above it
  may keep running with a dead `HubClient`. The asker eventually
  times out, but the release path is not audited.

A focused federation-path audit pass is warranted before relying on
multi-hub deployments. Until then, treat single-hub topologies as
the trusted configuration.

## Publisher persistence (the "who keeps your bytes online?" problem)

In a content-addressed network the bytes only exist where someone
has them on disk. A publisher with an ephemeral or flappy presence
(laptop, CI runner, anything residential) is a single point of
failure for everything they sign until the data has propagated to
nodes that stay online.

The current mitigation: `Edition::refresh` (`node/src/models/series.rs`)
eager-fetches the full inventory and spawns parallel object fetches
the moment a subscribed node sees an announcement. So in practice the
publisher only needs to stay online for the *propagation window* --
the time from announce to "first long-lived subscriber has the whole
edition." For the `get-samizdat` collection that means the publisher's
workstation needs to stay up minutes-to-hours after `samizdat
collection update` until the testbed has mirrored.

That's a workaround, not a fix. The publisher's network reachability
is still the bottleneck for a window each publish. The unresolved
real fix: a **paid pinning / mirror service tier**. A node that any
publisher can hire to eagerly subscribe to their series and pin its
content, taking the seeder role permanently so the publisher's
laptop becomes irrelevant after announce. Economics are the hard
part: who runs these nodes, how do they get paid, how is service
quality enforced. Probably ties into the identity dapp on Polygon
since payment + identity already live there.

Smaller, near-term items that orbit this:

- **Publisher-visible "is-current?" signal.** Today there is no
  clean way for a publisher to know when a subscriber has finished
  the eager fetch. CI publish workflows resort to black-box polling
  (curl the proxy URL of a known object). A `samizdat subscription
  is-current <series-key>` CLI or an HTTP admin endpoint on the
  node would make sync points explicit.
- **Pin-on-subscribe.** Even with eager fetch, the LRU eviction
  policy can later drop objects. A "this series is pinned, never
  evict" flag on the subscription record (with separate quota
  accounting) would let an operator dedicate a node to mirroring a
  set of series without juggling cache parameters.

## Blockchain (`blockchain/`)

- **Commit-reveal for name registration.** Mempool front-running
  lets watchers pre-empt a `register` for any unclaimed name. Switch
  to a commit -> wait N blocks -> reveal scheme.
- **On-chain name expiration.** Names registered with `registerWithTtl`
  are permanently squattable on-chain (the TTL only governs cache
  freshness, not ownership). Either add expiration after which the
  name returns to the pool, or document explicitly that registration
  is permanent.
- **Unicode normalization on identity keys.** Two visually-identical
  names with different code-point sequences are currently distinct
  on-chain. Normalize (NFC + confusable-detection) on the V1 path
  before forwarding to storage.
- **Pin Solidity pragma.** Currently `pragma solidity ^0.8.x`; pin
  to the exact compiler used for the deployed bytecode so future
  builds reproduce.
- **Node-side RPC trust.** The node verifies the configured RPC
  reports the expected chain ID but does not validate state with
  Merkle proofs. A malicious RPC can serve stale or forked state.
  Either document self-hosted RPC as the supported high-stakes
  configuration, or validate reads with `eth_getProof`.

## JS browser library (`js/`)

No known deferred items.

## Install pipeline (`samizdat-up`, `install/`, brew)

No known deferred items. The SCM wrapper on Windows ships in
`samizdat-up/src/install/windows.rs` via the hidden `daemon <role>`
subcommand, the matrix integration workflow runs on every push
(`.github/workflows/test-samizdat-up.yaml`), and the
`get-samizdat/.Samizdat.priv` git history has been audited (no blob
in either the submodule or the outer repo contains the key body or
the filename across all branches).

## Windows (`install/src/x86_64-pc-windows-gnu/`)

The post-overhaul backlog. Everything listed here is "next pass,"
not "broken today."

- **Log rotation.** The service appends to `samizdat-node.log` and
  `samizdat-node.err.log` forever. Add a size-based rotation
  (e.g. swap to `.1` at 50 MiB, keep one backup).
- **Code signing.** Sign `samizdat-installer.exe`,
  `samizdat-service.exe`, `samizdat-node.exe`, and `samizdat.exe`
  with an EV or OV cert so SmartScreen and the unsigned-driver
  warnings stop scaring users.
- **MSI / WiX alternative.** NSIS is fine for now but an MSI is the
  ticket for enterprise rollout (group policy, silent install).
- **`winget` and `chocolatey` packages.** Submit manifests pointing
  at the signed installer.

## Release pipeline

- **Wire up the testbed deploy.** The old
  `.github/workflows/deploy-testbed.yaml` was 100 lines of
  commented-out rsync+ssh+systemctl steps; it was deleted because
  triggering it did nothing. Replacement: a real
  `on: push: branches: [stable]` workflow that rsyncs new artifacts
  to the hub droplet and restarts the systemd units. The
  commented-out original is preserved in git history if useful as a
  starting point.

## Terraform (`terraform/`)

- **`digitalocean_firewall` resource.** Currently the droplet is
  exposed on every port DigitalOcean's default firewall doesn't
  block. Add an explicit allow-list (the hub QUIC/HTTP ports + SSH
  from a known CIDR).
- **SSH hardening.** Restrict to a known CIDR via the firewall
  above; disable password auth; enforce key auth.
- **`unattended-upgrades`.** Provision via cloud-init or a small
  Ansible step so security updates land without manual SSH.
- **`*.tfstate*` in `.gitignore`.** Today the TF Cloud backend means
  no local state, but a future `terraform state pull` would write an
  unprotected local file. Defensive ignore now.
