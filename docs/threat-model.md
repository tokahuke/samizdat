# Threat model

Read this before flagging anything as a "security" issue. Many findings have
non-obvious saving graces from how the pieces compose.

## Trust boundaries at a glance

```
   public internet
        |
        v
   samizdat-proxy        (HTTPS, ACME, public IP)
   strips Authorization
   strips Referer
   GET only
        |
        v
   samizdat-node:4510    (binds [::], deny_outside_requests middleware)
        ^
        |
   localhost only
        |
        +---- samizdat (CLI), bearer token from ~/.samizdat/access-token
        +---- browser pages, Referer-based trusted-context
        +---- other local processes (same uid)
```

```
   samizdat-node ----QUIC + tarpc---->  samizdat-hub        (operator-run, public IP)
                                              ^
                                              | hub-as-node, mutual
                                              v
                                        samizdat-hub        (other operators)
```

```
   samizdat-hub HTTP admin              binds 127.0.0.1 only
                                        operator-controlled
                                        no auth on the admin endpoints (see deferred)
```

## What is authenticated, and what isn't

### Node HTTP API (`http://127.0.0.1:4510`)

Three orthogonal auth mechanisms:

1. **`deny_outside_requests` middleware** rejects any non-loopback peer
   (`ConnectInfo<SocketAddr>` then `addr.ip().to_canonical().is_loopback()`).
   This is the outer cordon; nothing else matters until this passes.

2. **Bearer token** (`Authorization: Bearer <token>`). The token lives in
   `<data_dir>/access-token`, mode 0o600, never logged. The CLI reads it
   and uses it for every request. A constant-time compare gates it. Once
   bearer auth succeeds, the route's access-right requirement is bypassed.

3. **Referer-based trusted context**. A browser page loaded from
   `http://localhost:<port>/<entity>/...` automatically has its `Referer`
   header point at that entity. `entity_from_referrer` extracts the
   entity from the path and looks up its `Table::AccessRights` row. The
   rights stored there (granted via the `/_register` flow) gate which
   admin routes the page can call.

A small number of routes (`/_register`, `/_vacuum/*`) use a hybrid
"trusted context OR bearer" middleware (`authenticate_trusted_context`).

### Proxy (`https://samizdat.example.com`)

- Accepts GET only.
- Strips `Authorization`, `Referer`, and every other header from the
  incoming request.
- Forwards to the configured node URL (default `http://localhost:4510`).
- From the node's perspective every proxy-forwarded request is a
  loopback GET with no Referer, which `entity_from_request` returns as
  `entity = None`, which `do_authenticate_security_scope` resolves as
  `granted_rights = [AccessRight::Public]`.

The proxy CANNOT cause anything that requires a non-`Public` right.
Most admin routes are unreachable through the proxy by construction.

### Hub HTTP admin (`http://127.0.0.1:<http_port>`)

- Binds 127.0.0.1 (defense in depth on top of the loopback middleware).
- `deny_outside_requests` rejects non-loopback callers.
- **No auth on individual routes.** Adding an admin bearer token is on
  the deferred list. Today: any local process on the same host can hit
  the hub admin API. Acceptable for single-tenant operator hosts;
  unsafe on multi-tenant servers. See `hub/src/http/blacklisted_ips.rs`
  for the TODO.

### Node <-> Hub QUIC

- TLS server verification is **disabled** (`SkipServerVerification` in
  `common/src/quic.rs`). Trust does not flow through PKI; it flows
  through riddles, signatures on editions, and content-addressed
  identifiers.
- The node connects to a hub addressed by `<ip:port>` from the user's
  config. There is no certificate pinning; a MITM on the connection
  CAN substitute candidates and observe queries, but it CANNOT decrypt
  the riddle payloads (those are keyed by hashes neither the MITM nor
  the hub know) and CANNOT forge editions (those are Ed25519-signed by
  the series owner).
- Per-node throttling: a `tokio::sync::Semaphore` caps simultaneous
  queries; a `tokio::time::Interval` with `MissedTickBehavior::Delay`
  rate-limits queries-per-second. Both are per `HubServer` instance,
  which is per-connection, so each connected node has its own budget.
- Per-call replay resistance: every `Query` / `Resolution` /
  `EditionRequest` / `EditionAnnouncement` / `IdentityRequest` carries a
  `Hash` nonce. The hub records every observed nonce for
  `TOLERATED_AGE` (10 min); duplicates within that window are rejected.
  **It does not verify a fresh timestamp**, because the messages do not
  carry a signed timestamp; an attacker who captured a message can
  replay it once the eviction window passes. The throttle and semaphore
  are what bound abuse in practice. See
  `hub/src/replay_resistance.rs`'s module doc.

### Browser pages served by the node

A page at `http://localhost:4510/<entity>/...` runs in its own
same-origin context. Cross-origin pages can still issue requests to
`http://localhost:4510` via CORS, but:

- The node sets no CORS headers, so non-simple requests (PUT, DELETE,
  PATCH, anything with `Authorization` or `Content-Type: application/json`)
  are blocked at preflight.
- Simple POSTs with `text/plain` body DO bypass preflight. This is why
  `/_vacuum/*` was gated with `authenticate_trusted_context` (bearer or
  the `/_register` trusted context); a malicious page CANNOT call it
  cross-origin.

## Attack surface by attacker location

### Public internet (only proxy is reachable)

- GET arbitrary path -> proxy forwards as `granted=[Public]` GET to the
  local node. Bounded by what the node exposes at `Public` rights:
  content reads, basic resolution endpoints. Cannot mutate.
- The proxy's own admin: there is none. The proxy reads its config from
  CLI flags or a TOML file at startup and does not expose runtime
  configuration.
- ACME endpoints (`/.well-known/acme-challenge/`) are part of the
  `rustls-acme` flow; behave as expected.

### A peer hub or peer node over QUIC

- Cannot read content; everything riding the wire is opaque-encrypted
  with keys derived from content hashes.
- CAN inject candidates over `recv_candidate` (see deferred item:
  channel-id binding is not yet cryptographic). This poisons query
  results, not data on disk.
- CAN attempt to flood the throttle / semaphore. Bounded per-connection
  but not per-IP. A connection-count cap is on the deferred list
  (`hub/src/rpc/mod.rs` near `setup_connection`).
- CAN announce arbitrary editions over `announce_edition`. The receiving
  node binds the announced public key to the subscribed series
  (`node/src/system/node_server.rs::announce_edition`), so a malicious
  hub cannot trick the node into advancing an unrelated series.

### A malicious local web page in the user's browser

- Cannot read non-Public node endpoints unless it has a
  `Referer`-trusted-context grant for the right entity.
- Cannot CSRF mutating endpoints because of CORS preflight (Authorization
  header is non-simple).
- Can do simple POST with `text/plain` cross-origin. Routes that accept
  simple POST without auth are the surface; we audited them and gated
  the only sensitive ones (`/_vacuum/*`) with the trusted-context check.
- Cannot read `/_series-owners` (returns private key bytes) unless
  granted `ManageSeries`. See deferred item: `ManageSeries` is currently
  a flat right, not per-entity; a page granted `ManageSeries` for ANY
  entity can read every other entity's series owners' secrets. Local
  multi-tenant browser usage is not a supported configuration today.

### A malicious build script (`npm run build`, etc.) during `samizdat watch`

- The CLI's `commit()` walker now refuses to follow symlinks
  (`walk()` uses `symlink_metadata`). A build script that drops
  `dist/secrets -> ~/.ssh/id_ed25519` no longer leaks the SSH key.
- The build runs as the user; if it is malicious it can do anything the
  user can do (rm -rf, etc.). The symlink refusal closes one specific
  exfiltration path.

### A process on the user's machine (same uid)

- Can read `~/.samizdat/access-token` and use it to call any node
  endpoint. This is in-band of the threat model; the access token IS
  the local trust root for admin operations.
- Can read `.Samizdat.priv` for any project under the user's home
  directory. The file is mode 0o600 so other-uid processes are kept
  out.
- Can hit the hub admin endpoints. See deferred admin-token TODO.

## Known not-bugs

These look like bugs but are intentional in the model. Do not "fix" them.

- **QUIC server verification is off.** Trust flows through hashes and
  signatures, not PKI. Adding cert verification would be a no-op
  (there are no trusted CAs in this network) and would add a misleading
  layer of "real TLS" semantics.
- **Identical chunk hashes are not de-duplicated per-object.**
  `ObjectChunkRefCount` is keyed by chunk hash globally; an object with
  repeated chunks (e.g. zero-padding) increments the same refcount many
  times. Saturating at `i32::MAX` is correct, but the counter is
  designed for a different invariant than per-object dedup.
- **The hub sees that some peer claims to have content matching a
  riddle.** It cannot decode which content. This is by design;
  unrelating the matcher from the matched content is the whole point of
  the riddle layer.
- **Vacuum can race chunk imports for the first few seconds after a
  crash.** A startup sweep in `vacuum::sweep_crash_leaked_chunks` runs
  BEFORE any task that imports chunks is spawned, so the race is
  one-shot and the sweep is safe.

## Known limitations (deferred but not bugs)

See `docs/audit-history.md` for the deferred list and where each
mitigation TODO lives in the source. Highlights:

- No admin token on hub HTTP API.
- No per-IP connection cap on hub QUIC accept.
- No cryptographic binding of `ChannelId` to a specific responder, so
  a connected peer can inject candidates for queries it was not
  assigned to.
- Replay resistance window is per-nonce only; no fresh-timestamp check.
- Matcher's per-call cleanup-task spawn is unbounded (perf, not
  correctness).
