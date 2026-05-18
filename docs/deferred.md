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

1. **Cryptographic `ChannelId` binding.** HMAC over
   `(client_addr, peer_id, server_secret)`. Closes the
   `recv_candidate` injection on `hub/src/rpc/hub_server.rs` and
   `hub/src/rpc/hub_as_node.rs` in one stroke (H9 / SP federation).
2. **Hub admin token middleware** (`SAMIZDAT_HUB_ADMIN_TOKEN`).
   Closes H4 (`/blacklisted-ips` unauth). Add the matching DELETE
   route for blacklist removal while you are there.
3. **Per-IP connection cap in hub QUIC accept** (H11) and move
   `try_acquire` ahead of `accept_bincode_transports` (H12). A flood
   currently allocates transports before being rejected.
4. **Replay-resistance with signed timestamps.** Add an authenticated
   timestamp to messages and verify freshness in `check`. Today the
   only protection beyond the per-nonce dedup is the throttle.
5. **`Riddle::riddle_for` padding (P14).** Leaks message length
   (e.g. IPv4 vs IPv6). Wire-format-breaking; bundle with the next
   protocol break. Existing `// TODO: ... Need padding!` in
   `common/src/riddles.rs`.
6. **`Matcher` cleanup-task bound (B13).** Each
   `Matcher::{expect,arrive}` spawns a 10s cleanup task. Use
   `tokio::time::DelayQueue` instead. Perf, not correctness.
7. **Per-entity scoping of `ManageSeries` (B15 root cause).** Then
   strip private-key bytes from list responses.
8. **Hub HTTP body size cap.** Only matters if the proxy is ever
   pointed at a remote node; loopback-only deployments do not need
   it.

## Second-pass deferred

- **SP-D1 / Reset-trigger drops loser direction (silent candidate
  loss across reconnect).** `node/src/system/mod.rs`
  `HubConnectionInner::connect` uses `future::select` on the two
  reset receivers; when one fires, the surviving direction keeps
  running on the dying QUIC connection. Same shape in
  `hub_as_node::connect`. The naive fix (`connection.close()` on
  either reset) regresses in flaky-network scenarios because it
  forces a full re-handshake on every tarpc dispatch wobble. The
  right fix has the new connection INHERIT the old
  `candidate_channels` so in-flight queries see candidates while
  new tarpc instances spin up. Non-trivial refactor.

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

- **`samizdat-up install` on Windows.** Currently a stub that bails;
  the SCM registration logic in the deleted
  `install/src/x86_64-pc-windows-gnu/node/samizdat-service` needs to
  be ported into `samizdat-up/src/install/windows.rs`. Two design
  choices to pick from: (a) make `samizdat-node.exe` SCM-aware
  itself by calling `windows_service::service_dispatcher::start`
  from its main, so `sc.exe create` is enough; (b) have samizdat-up
  also act as the SCM wrapper, with a `samizdat-up daemon <role>`
  hidden subcommand that SCM points at. (a) is cleaner; (b) reuses
  the old wrapper pattern.
- **Matrix integration test workflow** for samizdat-up
  (`.github/workflows/test-samizdat-up.yaml`). Per the plan: ubuntu
  + macos + windows runners, each `cargo build`, `samizdat-up
  install node --from file://`, OS-native service check,
  `samizdat-up uninstall --purge`, OS-native gone check. The
  Windows path of this workflow is what would catch regressions in
  the windows branch above. The maintainer has no Windows machine
  to test on, so this matrix IS the Windows test.
- **Homebrew formula `sha256` + versioned URL.** The current formula
  pulls from `latest/`, no sha pin. To pass `brew install` cleanly
  it needs version + sha256 baked in per release. Wire into the
  publish workflow: after the sha of the macOS samizdat-up tarball
  is known, generate a new `Samizdat.rb` and push to the
  `homebrew-samizdat` tap repo.
- **Switch `get-samizdat` install scripts to HTTPS.** (Already done
  in `install.sh.template`; remove from this list once verified.)
- **`get-samizdat/.Samizdat.priv` history verification.** The
  install collection has a series key; verify nothing leaked it via
  `git log -p -- '.Samizdat.priv'` or equivalent.

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
