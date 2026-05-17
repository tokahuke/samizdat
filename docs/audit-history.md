# Audit history

A multi-pass bug audit of the workspace was conducted. Findings
are tagged by crate (P# = common primitives, N# = node, B# = node behavioral,
H# = hub, F# = proxy, plus a handful from cli). The most important rule:
**do not re-flag anything on the deferred list without reading why it was
deferred**. Several findings looked scary in isolation but have non-obvious
saving graces from the larger threat model; see `docs/threat-model.md`.

## Method

For each crate we ran a structured pass: spawn an audit agent with a
focused remit, critically filter the results against the threat model
(false positives are expensive), apply targeted fixes, and write tests
where they would have caught the original bug. Style/aesthetic clippy
lints were ignored throughout; only correctness, footgun, perf, and
security lints were acted on.

## Findings by crate

### `common` (primitives)

| ID | Issue | Status |
|----|-------|--------|
| P1 | `MerkleTree::is_proved_by` always returned `false`: it compared the just-computed parent against the sibling, which never matches for a valid proof. Compounded by `proof_for` walking levels root-first while `is_proved_by` expected leaf-first. The function had no tests and no real callers. | Fixed |
| P2 | `TransferCipher::decrypt` silently swallowed AEAD authentication failures via `.ok()`. Callers parsed corrupted bytes downstream. | Fixed: returns `Result`, propagates through `Encrypted::decrypt_with` and `OpaqueEncrypted::decrypt_with`. |
| P3 | `PrivateKey: Debug + Display` printed the raw secret bytes. One `dbg!` or `tracing::error!("{priv:?}")` would leak the key. | Fixed: redacted Debug/Display; new `reveal_base64()` for legitimate persistence. |
| P4 | `MerkleTree::proof_for(index)` accepted `index == len()` (off-by-one), producing a bogus proof one past the last leaf. Masked by P1. | Fixed: `>= len()` rejection. |
| P5 | `PrivateKey` did not zeroize on drop. | Fixed: `ZeroizeOnDrop` impl. |
| P6 | AES-256 key derivation was right-zero padding of SHA3-224 hash, not a KDF. Effective entropy fine today (224 bits), but brittle if `HASH_LEN` is ever shrunk. | Fixed: HKDF-SHA256 with domain-separation tag. Wire-format breaking change; OK because pre-launch. |
| P7 | `KeyedChannel::recv_stream` evicted prior listeners silently and the dropped stream's `Drop` removed whoever then owned the slot. | Fixed: per-listener generation counter; `Drop` only removes its own slot. |
| P8 | `KeyedChannel` used `mpsc::unbounded`. | Fixed: bounded `mpsc::channel(1024)`; overflow warns and drops. |
| P9 | All `Table` accessors panicked on LMDB errors (MAP_FULL, OS I/O, MDB_READERS_FULL). | Fixed: full Result propagation across `Table`, `TableRange`, `TablePrefix`, `Migration`. Touched ~16 caller files. |
| P10 | `MerkleTree::from(Vec::new())` produced a tree whose `root()` panicked inside callers. | Fixed: `try_from_leaves` (returns Option) and a clear panic message for the infallible `From`. |
| P11 | `Hash::from_str` error said "expected 64 bytes" instead of 28. | Fixed. |
| P12 | `From<i64> for Hash` was a footgun (puts 8 bytes, rest zero). No callers. | Removed. |
| P13 | `Hint::new` panicked out of bounds for `length > HASH_LEN` (instead of for `length >= 256`). | Fixed: `length <= HASH_LEN`. |
| P14 | `Riddle::riddle_for` leaks message length (e.g. IPv4 vs IPv6). | **Deferred.** Wire-format-breaking; bundle with a future protocol change. Pre-existing `// TODO: ... Need padding!` comment in `common/src/riddles.rs`. |
| P15 | "expected 244" typo in `PatriciaProof::is_in`. Should be 224. | Fixed. |

Plus: `csprng()` was reseeding ChaChaRng from 64 bits of entropy; fixed to seed from 32 bytes via `from_seed`.

Plus: full DB error propagation pass (P9). The `Table` trait and helpers in
`common/src/db.rs` now return `Result<_, crate::Error>` from every
operation. Closures that used to deserialize via `expect()` now return
`Result` and propagate cleanly.

Plus: a `TestDb<T>` harness in `common::db::test_harness` (behind the
`test-helpers` feature) so DB-backed tests can run without polluting the
real database singleton.

### `node` (known + new findings)

| ID | Issue | Status |
|----|-------|--------|
| N1 | Partial-import leak in `ObjectRef::do_import`: chunks written to LMDB as they arrive, abandoned on error. Vacuum couldn't see them (no refcount entry). | Fixed: best-effort rollback on Err inside `do_import`, plus `vacuum::sweep_crash_leaked_chunks` startup-only pass that deletes chunks-without-refcount before any import task can race. |
| N2 | `<data>/access-token` written with default umask (0644). | Fixed: mode 0o600 on Unix. |
| N3 | Access token logged at `tracing::info!`. | Fixed: log only the length. |
| N4 | Access-token compare was non-constant-time. | Fixed: `subtle::ConstantTimeEq`. |
| N5 | `Matcher::{expect,arrive}` asserted on `ChannelId` collisions; reachable via the 32-bit id. | Fixed: log+continue, plus widened `ChannelId` to u64 across the workspace. |
| N6 | After the DB refactor, `do_authenticate_security_scope` swallowed DB errors via `.ok().flatten()`. | Fixed: fail-closed; logs the error and returns `InsufficientPrivilege`. |
| B1 | `/_vacuum/*` endpoints unauthenticated. Simple POST with `text/plain` body would bypass CORS preflight. | Fixed: gated with `authenticate_trusted_context` middleware. |
| B2 | `/_kvstore` PUT was Public scope with `DefaultBodyLimit::disable()`. | False alarm: PUT triggers CORS preflight, blocked from cross-origin pages. Comment notes the trust-boundary reasoning. |
| B3 | `announce_edition` did not bind announced public key to the matching subscription's key. A hub could wrap a B-signed edition under an A-targeted announcement. | Fixed: explicit `edition.public_key() == &subscription.public_key` check post-decrypt. |
| B4 | `SeriesRef::advance` (peer-driven path) skipped bookmark accounting. Subscribed-series objects were not Reference-pinned and vacuum could drop them. | Fixed: mirrored the bookmark dance from `SeriesOwner::advance`. |
| B5 | Edition `Table::Editions` key was at second granularity. Same-second editions overwrote each other. No monotonicity check. | Fixed: microsecond precision in key, monotonicity + idempotency checks in `SeriesRef::advance`, new `crate::Error::StaleEdition`. |
| B6 | `Edition::refresh` advanced and marked the series fresh BEFORE fetching the inventory. A hub could DoS a subscription for one TTL by announcing valid metadata and withholding the inventory object. | Fixed: fetch first, then commit advance + refresh atomically. |
| B7 | Vacuum's `drop_orphan_chunks` was a read-then-write TOCTOU: a concurrent import could bump a refcount between the scan and the delete. | Fixed: scan + delete inside a single `writable_tx`; refcount entry deleted alongside chunk. |
| B8 | `Bookmark::clear` zeroed Reference refcounts on object drop. Looked dangerous but `vacuum`'s `is_bookmarked` guard meant the path was unreachable in practice. | Fixed conservatively: `ObjectRef::drop_if_exists_with` now only clears `User`, leaves `Reference` to decay naturally. Comment documents the saving grace. |
| B9 | `MergeOperation::Increment(i16)` wrapped silently on overflow. | Fixed: widened to `i32`, `saturating_add` semantics. |
| B10 | `Hubs::get_edition` did not filter candidate editions by series public key. The downstream `advance` rejected them, but the cheap filter is hardening. | Fixed: one-line filter. |
| B11 | Identity-dapp provider lacked chain-ID verification; `ttl as i64` could wrap. | Fixed: verify chain ID 137 (Polygon) on each `get`; saturate `ttl` cast at `i64::MAX`. |
| B12 | `recv_item` inserts the collection-item row BEFORE the underlying object finishes downloading. | False alarm. `resolve_object` re-checks existence and falls through to the network query; `vacuum::drop_dangling_items` reaps items whose object never arrives. Documented inline. |
| B13 | `Matcher::{expect,arrive}` spawn a 10s cleanup task per call. Unbounded under sustained load. | **Deferred.** Perf, not correctness. TODO comment with `tokio::time::DelayQueue` as the upgrade path. |
| B14 | `ObjectMessage::validate` builds the Merkle tree from peer hashes BEFORE the size cap check; `max_content_size * 1_000_000` is unchecked. | Fixed: size check first; `saturating_mul`. |
| B15 | `GET /_series-owners` returns the private key bytes. | **Mostly false alarm.** Proxy strips auth (unreachable); `ManageSeries` is already admin-level; CLI explicitly depends on the shape for `series show` backup. Real concern is the flat (not per-entity) right scope. Comment inline with the design constraint. |

Plus: the `node/src/system/mod.rs:182` bug where the configured `rpc_context`
was built with a deadline and never used (a fresh `context::current()` was
passed to the RPC, silently dropping the deadline). Caught by clippy.

### `hub`

| ID | Issue | Status |
|----|-------|--------|
| H1 | `HubServer::new` configured a `call_throttle` with `MissedTickBehavior::Delay`, then stored a fresh `interval(...)` with default `Burst`. Per-node rate limit was effectively absent under load. | Fixed. |
| H2 | `candidates_for_resolution` does `pop().expect("non-empty resolution")`. Both callers guard against empty, so the expect is unreachable today. | Documented inline; not fixed. |
| H3 | Hub HTTP admin server bound `Ipv6Addr::UNSPECIFIED`. | Fixed: bind `Ipv6Addr::LOCALHOST`. |
| H4 | `/blacklisted-ips` POST has no auth, no DELETE route, no audit. | **Deferred.** Combined with H3 (loopback-only) the realistic threat is bounded to co-tenant processes. TODO inline. |
| H5 | `Id::to_bytes(desc=false)` used `to_le_bytes`, which doesn't sort lexicographically. Latent: no caller uses `DESC = false` today. | Fixed: `to_be_bytes` in both branches. |
| H6 | `REPLAY_RESISTANCE` was a global tokio `Mutex<ReplayResistance>`. Every RPC serialised through it and did two LMDB transactions per check. | Fixed: `&self` method, single `writable_tx` does `has` + `put` atomically, no outer Mutex. |
| H7 | Replay-resistance does not verify a fresh timestamp; the `now` was recorded but not compared against anything in the message. | **By design with caveats**; module doc rewritten to be honest about the limit. Real safeguards against repeated abuse are throttle + semaphore. |
| H8 | `generate_id` produced duplicates on backward clock jump. | Fixed: `(last + 1).max(raw_id)` with saturating_add and clock-pre-epoch fallback. |
| H9 | `hub_as_node::recv_candidate` is unauthenticated. Same channel-id-binding issue as the client-facing path. | **Deferred.** TODO inline near both `recv_candidate`s; the binding work is one ticket. |
| H10 | `BlacklistedIp::get_all` panicked on bad rows. | Fixed: propagate as `crate::Error`. |
| H11 | No per-IP connection cap; a single host can fill `max_connections`. | **Deferred.** TODO inline at `setup_connection`. |
| H12 | `try_acquire` on the connection semaphore happens AFTER `accept_bincode_transports`. A flood allocates transports before being rejected. | **Deferred.** Same TODO. |
| H13 | Initial tick of `Interval` fires immediately, allowing a burst of `max_queries` at startup. | Acceptable. |

Plus, from clippy:
- `generate_id`'s `duration_since(UNIX_EPOCH).expect("Time went backwards")`
  crashed the hub on backward clock past 1970. Fixed: `unwrap_or_default()`.
- `node_sampler::sample` cast a possibly-NaN `f64` priority to `i64`,
  which saturates to 0 and tied broken nodes with healthy ones. Fixed:
  clamp non-finite to `i64::MIN`.
- `LAST_ID` static promoted from function-local to module-level for
  discoverability.

### `cli`

| ID | Issue | Status |
|----|-------|--------|
| 1 | `--private-key <KEY>` accepted on argv: visible in `ps`, shell history, `/proc/<pid>/cmdline`, audit logs. | Fixed: renamed to `--private-key-file <PATH>` in `import` and `series new`. |
| 2 | `.Samizdat.priv` written via `fs::write` (default umask, 0644 typical). | Fixed: new `write_priv_file` helper, mode 0o600 on Unix. |
| 3 | Watch-mode WebSocket had no Origin check. | Fixed: `accept_hdr` with loopback-origin validation; missing Origin still allowed for native ws clients. |
| 4 | WebSocket uses `thread::spawn` per connection plus an unbounded `Vec` of senders. | **Deferred.** Loopback-only; perf hardening for later. TODO inline. |
| 5 | ANSI escapes in node responses get printed raw. | Trust boundary is the local node. Comment inline. |
| 6 | No response-body size limit. | Trust boundary; documented inline. |
| 7 | `http://localhost:{port}` could be redirected by `/etc/hosts`. | Fixed: `http://127.0.0.1:{port}` everywhere. |
| 8 | `commit()` walker followed symlinks. A malicious build script could drop `dist/secrets -> ~/.ssh/id_*` and have it published. | Fixed: `symlink_metadata`, refuses symlinks, only walks dirs and reads regular files. |
| 9 | Watch-mode rebuild predicate is brittle if `base = "."` (or any ancestor). | Documented inline. |
| 10 | Walk-then-read race in `commit()`. | Subsumed by #8 (the symlink fix closes the practical exploit). |
| 11 | `samizdat series rm <name>` had no confirmation. | Fixed: `--yes`/`-y` flag, interactive prompt otherwise, refuses without TTY + `--yes`. |
| 12 | Project `name` flowed raw into the TOML template (Askama `.txt` extension = no escaping). | Fixed: `validate_project_name` rejects everything outside `[A-Za-z0-9._/-\ ]`, caps length at 128. |
| 13 | `samizdat init` banner printed `PrivateKey(<redacted>)` after the Display redaction. | Fixed: call `reveal_base64()` explicitly. |
| 14 | `tracing::info!` logged response bodies; `/_series-owners` responses include private keys. | Fixed: `SENSITIVE_BODY_ROUTES` allow-list + `redact_if_sensitive` helper used by every HTTP verb. |
| 15 | `expect()` on non-UTF-8 paths and `pwd.iter().last()` quirks. | Skipped; edge cases unlikely in practice. |

Plus from clippy:
- `pwd.iter().last()` -> `pwd.iter().next_back()` (`DoubleEndedIterator`).
- `splitn(2, ...).nth(1)` -> `split_once(...)`.

### `proxy`

| ID | Issue | Status |
|----|-------|--------|
| F1 | `do_proxy` had `expect("always starts with /, right?")` and a `todo!()` arm. A crafted path could panic the request thread. | Fixed: explicit match, no panics. |
| F2 | ACME state stream's `None` branch silently broke the renewal loop. Cert renewal stopped silently. | Fixed: loud `tracing::error!` on stream end. |
| F3 | CSS namespace prefix used 16 bits of randomness. | Fixed: 32 bits. |
| F4 | `mime == TEXT_HTML_UTF_8` missed common charset variants. | Fixed: compare on `type_()`/`subtype()`. |
| F5 | `tracing::info!` of full CLI args at startup included `owner` email (PII). | Demoted to `debug!`. |

## Tests added by the audit

- `common::hash::tests`: merkle round-trip at multiple sizes, wrong-leaf
  rejection, off-by-one rejection, empty-tree handling, error-message
  regression. (7 tests)
- `common::cipher::tests`: round trip, wrong-key/tampered detection,
  end-to-end `Encrypted` and `OpaqueEncrypted`. (5 tests)
- `common::pki::tests`: Debug/Display redaction, `ZeroizeOnDrop` bound,
  signed round-trip and wrong-key rejection. (4 tests)
- `common::keyed_channel::tests`: basic send-recv, drop-of-replaced-listener,
  drop-of-sole-listener, bounded channel. (4 tests)
- `common::riddles::tests`: riddle resolves, message riddle round-trip,
  wrong-secret rejection, hint max length, hint oversized rejection. (5 tests)
- `common::db::tests`: harness round trip, transform-error propagation,
  range-for-each error propagation, closure-error propagation. (4 tests)
- `node::db::tests`: `MergeOperation` saturating arithmetic, eval_on_zero,
  past-i16-cap accumulator. (5 tests)
- `node::models::series::tests`: ownership generate, edition validation,
  edition key includes microseconds, stale-edition rejection, idempotent
  re-advance, cross-key forgery rejection. (7 tests)
- `hub::models::tests`: monotonic id under burst, BE id sort order. (2 tests)
- `hub::replay_resistance::tests`: accepts then rejects replay, distinct
  nonces independent. (2 tests)

Total: 51 unit tests across the workspace (5 pre-existing + 46 added).

## Things to never re-flag

- **Merkle proof verification** is now correct. Don't trust an
  agent who tells you the math is off.
- **`PrivateKey: Debug` is intentionally redacted**. Use `reveal_base64()`
  explicitly to persist.
- **`TransferCipher` uses HKDF-SHA256 now**, not zero-padding.
- **The hub admin plane binds 127.0.0.1**, not `::`.
- **`ChannelId` is u64**, not u32. `ChannelId::random()` is the
  allocator.
- **DB layer returns `Result` everywhere**. Don't propose adding
  `expect()` or `unwrap()` to "simplify".
- **The replay-resistance freshness check that "doesn't exist"** is by
  design, not an oversight; the messages don't carry a signed
  timestamp.
- **The candidate's `socket_addr` is intentionally unbounded.** The
  federation is a recursive graph (see `architecture.md` and
  `threat-model.md`). At any given hub the "responding peer" is the next
  hub in the chain, not the actual content holder several hops away.
  Any agent that proposes binding `candidate.socket_addr.ip()` to the
  responding peer's connection IP is **wrong**; that fix breaks
  federation. The acceptable mitigations live elsewhere (cryptographic
  channel-id binding on the deferred list; making the dial itself
  unexploitable as a side channel by other means).

## Things still on the deferred list

In priority order if anyone wants to pick them up:

1. **Cryptographic `ChannelId` binding** (HMAC over
   `(client_addr, peer_id, server_secret)`). Closes:
   - `recv_candidate` injection on both hub_server.rs and hub_as_node.rs.
2. **Hub admin token middleware** (`SAMIZDAT_HUB_ADMIN_TOKEN`). Closes:
   - H4 (`/blacklisted-ips` unauth).
   - Adds DELETE route for blacklisted IPs.
3. **Per-IP connection cap in hub QUIC accept** (H11), and move the
   `try_acquire` ahead of `accept_bincode_transports` (H12).
4. **Replay-resistance with signed timestamps**: add an authenticated
   timestamp to the messages and verify freshness in `check`. Currently
   the only protection beyond the per-nonce dedup is the throttle.
5. **`Riddle::riddle_for` padding** (P14): leaks message length.
   Bundle with whatever the next wire-format break is.
6. **`Matcher` cleanup-task bound** (B13): use `tokio::time::DelayQueue`.
7. **Per-entity scoping of `ManageSeries`** (B15 root cause). Then
   strip private-key bytes from list responses.
8. **Hub HTTP body size cap** if the proxy is ever pointed at a remote
   node.

## Second pass: high-level / protocol findings

A separate pass focused on node-to-hub lifecycle, the protocol itself, the
file-sharing algorithm, and hub-to-hub federation. Findings carried tags
"second-pass" (SP) in the working list.

### Fixed in this pass

- **SP1 / Hub QUIC accept panic on transient DB.** `hub/src/rpc/mod.rs`
  blacklist lookup used `.expect("db error")` — the one site the P9
  Result-propagation sweep missed. A transient `MAP_FULL` /
  `READERS_FULL` would silently kill the QUIC accept loop, leaving the
  hub up but no longer accepting new connections. Now propagates as a
  fail-closed drop with a tracing::error log.
- **SP2 / `is_fresh` panic on huge TTL.** `node/src/models/series.rs`
  `chrono::Duration::from_std(...).expect(...)` crashed the read path
  if a series owner shipped a single edition whose `ttl` exceeded
  `chrono::Duration::MAX`. Now saturates at
  `chrono::Duration::max_value()`.
- **SP3 / Chunk-distribution non-termination.** `Hashes::is_done` used
  `==`; under torrent-style multi-peer delivery `received` overshoots
  `original_size` and equality never holds, so candidate tasks busy-loop
  sending empty `GetChunks` until QUIC drops. Changed to `>=`.
- **SP4 / Asymmetric semaphore = silent half-open connection.**
  `hub/src/rpc/mod.rs::setup_connection` acquired client and server
  permits in separately-spawned tasks. If one was exhausted the other
  still ran, producing a connection that handshakes successfully but
  silently can't service queries (`NoReverseConnection` on every
  `do_query`, or no receiving server). Now acquires both up front
  before negotiating transports; either failure closes the QUIC
  connection cleanly.
- **SP5 / Permanent-pin via clock-forward.** `SeriesRef::advance` had
  monotonicity (B5) but no upper bound on `edition.timestamp`. A
  compromised series owner could publish one edition with
  `timestamp = now + 100y` and lock subscribers to it indefinitely.
  Added an `EDITION_CLOCK_SKEW` (5 minute) bound at the module top.
- **SP6 / FED-1: `hub_as_node::resolve` unsupervised forwarding task.**
  The spawned forwarding task had no cancellation: when the upstream
  partner went away, it kept producing candidates and pushing them
  into a dead RPC. Now breaks the loop on the first `recv_candidate`
  Err.
- **SP7 / Refresh-DoS cooldown on the node side.** A malicious hub
  could mint endless `EditionAnnouncement`s for any known-public series
  key; each one cost a task spawn + Ed25519 verify + DB read on the
  receiver before being rejected. Added per-series 30s cooldown in
  `node/src/system/node_server.rs` (`LAST_REFRESH_ATTEMPT` map gated by
  `ANNOUNCE_COOLDOWN`).
- **SP8 / FED-2: `HubAsNodeServer` methods skip throttle.** Client-facing
  `HubServer` had a per-connection throttle and concurrency cap; the
  hub-as-node path had none. A misbehaving partner hub could blast any
  of the four `Node` methods at line-rate. Now mirrors `HubServer`'s
  `throttle` helper, gated by `max_queries_per_hub` /
  `max_query_rate_per_hub`.

### Deferred from this pass

- **SP-D1 / Reset-trigger drops loser direction (silent candidate loss
  across reconnect).** `node/src/system/mod.rs::HubConnectionInner::connect`
  uses `future::select` on the two reset receivers. When one direction
  fires, the surviving direction keeps running on the dying QUIC
  connection. Affects both node-hub and hub-hub
  (`hub_as_node::connect`). The naive fix (`connection.close()` on either
  reset) regresses in flaky-network scenarios because it forces a full
  re-handshake on every tarpc dispatch wobble; the right fix has the
  new connection INHERIT the old `candidate_channels` so in-flight
  queries can still see candidates while the new tarpc instances spin
  up. Non-trivial refactor; not done here.

### Confirmed not bugs

- **SP-N1 / Reconnect holds the write lock across the backoff loop.**
  Looked like an availability issue (callers block waiting for new
  connection), but the `Hubs::query` layer fans out to all configured
  hubs via `buffer_unordered` and returns the first `Ok(found)`. Single-
  hub deployments DO have indefinite hangs on the down hub; the design
  choice is "transparent wait" rather than "fast fail," consistent with
  the structured `ConnectionStatus` state machine. Document the
  multi-hub deployment as the supported configuration.
- **SP-N2 / Asker doxing via candidate injection.** The federation is a
  recursive graph; at any hub the "responding peer" is the next hub in
  the chain, not the actual content holder several hops away.
  `candidate.socket_addr` is intentionally unbound. Reduces to a small
  reflection primitive (handshake at a chosen victim IP, no content
  leaks because `NonceMessage::recv_negotiate` fails). See
  `threat-model.md` for the structural explanation.

## Under-audited areas (known unknowns)

The second pass mapped the node-hub connection lifecycle, the
overall protocol, and the file-sharing algorithm. The hub-to-hub
federation path was touched only tangentially. Likely sources of
undiscovered bugs:

- **Multi-hop candidate routing.** Every hop adds a `channel_id`
  indirection in its `candidate_channels` map. Cleanup paths when an
  intermediate hub disconnects, or when the asker times out partway
  through the chain, are not well exercised.
- **Deadline propagation across hops.** Whether the tarpc
  `Context.deadline` flows correctly through `Hub::resolve` ->
  `HubAsNodeServer::resolve` -> next-hop `resolve`, or gets reset to
  `context::current()` at any hop, is not verified. If reset, malicious
  hubs can extend deadlines. If propagated, deep-hop responses may be
  silently discarded.
- **Replay-resistance under cycle.** `ReplayResistance::check` is keyed
  by the message nonce; cycles in the federation graph ought to be cut
  by it because the same nonce is preserved across hops. Worth a
  property test.
- **`HubAsNodeServer::recv_candidate` has no `throttle`.** The
  client-facing `HubServer::recv_candidate` goes through `throttle`;
  the partner side does not. A malicious partner can blast
  `recv_candidate` faster than a normal node could.
- **Forwarding-task lifecycle on partner disconnect.** When a partner
  hub mid-chain drops mid-fan-out, the forwarding tokio task above it
  may keep running with a dead `HubClient`. The asker eventually times
  out, but the task's release path is not audited.

A focused federation-path audit pass is warranted before relying on
multi-hub deployments. Until then, treat single-hub topologies as the
trusted configuration.

All have TODO comments in the source with the file and a one-line
explanation; grep for `TODO(`.
