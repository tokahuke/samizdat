# Samizdat architecture

Samizdat is a peer-to-peer, content-addressed publishing network. Users publish
versioned collections of files; consumers fetch them by series identifier or by
content hash. Routing happens through one or more **hubs** which act as
matchmakers but never see the content itself.

This document describes the system top-down, then crate-by-crate, then the
cross-cutting primitives (riddles, content addressing, transport, identity).

## 30-second mental model

```
   publisher               consumer
   (CLI + node)            (browser + node)
        |                          |
        +-----> samizdat-node <----+         (per-user, localhost)
                    |  ^
                    |  |  QUIC + tarpc, mutually-distrustful
                    v  |
                samizdat-hub                 (operator-run, public)
                    |  ^
                    |  |  QUIC + tarpc, hub-as-node federation
                    v  |
                samizdat-hub                 (other operators)
```

For public web access without running a node, a `samizdat-proxy` serves the
local node's HTTP API over HTTPS with Let's Encrypt certs.

## Domain model

Four nested concepts; each is content-addressed.

### Object

A file. The byte stream is split into 256 KB chunks; each chunk is hashed
(SHA3-224, 28 bytes); the chunk hashes form the leaves of a Merkle tree whose
root *is* the object's identity. An `ObjectHeader` (MIME type, draft flag,
nonce) is prepended to the byte stream before chunking, so the same bytes with
a different MIME live at a different content hash. See `common/src/hash.rs`
for the Merkle implementation and `node/src/models/object.rs` for the storage
layer.

### Collection

A named directory of objects. Internally a `PatriciaMap<path-hash, object-hash>`
keyed by `Hash::from_bytes(path)`. The root of the Patricia tree IS the
collection's identity. Each `(collection_hash, item_path)` produces a
`Locator` whose hash is also content-addressed, so consumers can resolve
"give me `/index.html` in collection X" without revealing either name to the
hub. See `common/src/patricia_map.rs` and `node/src/models/collection.rs`.

### Edition

A signed pointer from a stable identity (series public key) to a specific
collection at a specific time. `EditionContent { kind, collection, timestamp, ttl }`
is bincode-serialised and signed with the series's Ed25519 keypair. Editions
are `Base` (replace) or `Layer` (additive, with fallthrough to previous
editions). See `node/src/models/series.rs`.

### Series

A stable identity = an Ed25519 public key. Anyone who can sign with the
matching private key can publish new editions. A consumer "subscribes" to a
series; their node periodically queries the hubs for the latest edition and
fetches the inventory + new objects.

### How the layers compose

```
       Series (pubkey)
           |
           v
       Edition (signed { collection_root, timestamp, ttl, kind })
           |
           v
       Collection (Patricia root of path->object_hash)
           |
           v
       Object (Merkle root of chunk hashes)
           |
           v
       Chunks (256 KB content-addressed blobs)
```

## Crate by crate

### `common`

The shared library. Contains:

- **`hash`**: `Hash` (28-byte SHA3-224) plus `MerkleTree` and `InclusionProof`.
- **`patricia_map`**: bit-trie of `Hash -> Hash` used for collections.
- **`pki`**: `PrivateKey` (zeroizing, redacted Debug/Display) and `Key`
  (public), plus `Signed<T>`.
- **`cipher`**: `TransferCipher` (AES-256-GCM-SIV, key derived via
  HKDF-SHA256 from a content hash); `OpaqueEncrypted` and `Encrypted<T>`.
- **`riddles`**: `Riddle`, `MessageRiddle`, `Hint`. See the riddle section
  below.
- **`rpc`**: tarpc service definitions for `Hub` and `Node`. The same types
  are used for client-server and hub-as-node federation.
- **`quic`**: QUIC endpoint setup with `SkipServerVerification`. TLS in
  Samizdat is deliberately untrusted; trust flows through the hash/signature
  layer instead.
- **`transport`**: bincode-codec wrappers over QUIC bi-streams.
- **`address`**: `ChannelId` (u64) and `ChannelAddr` (peer_addr + channel_id).
- **`db`**: the LMDB wrapper used by `node` and `hub`. Defines the `Table`,
  `TableRange`, `TablePrefix`, `Migration` traits. All accessors return
  `Result`; see `docs/conventions.md`.
- **`db::test_harness`**: `TestDb<T>` for unit tests behind the
  `test-helpers` feature.

### `node`

The per-user peer. Each samizdat user runs one. It:

- Stores objects, collections, editions, series, subscriptions in LMDB.
- Talks to one or more hubs over QUIC + tarpc (both directions:
  "node as client" for `query`/`get_edition`/`announce_edition`, "node as
  server" so the hub can push candidates back).
- Serves a localhost HTTP API used by the CLI and by browser pages
  (`/_objects`, `/_series-owners`, `/_subscriptions`, etc.).
- Authenticates browser clients via a `Referer`-based "trusted context" or a
  bearer access token; see `docs/threat-model.md`.
- Runs a vacuum daemon that GCs objects whose `is_bookmarked` is false.

Key modules:

- `models/{object,collection,series,subscription,bookmark,hub}.rs`
- `system/`: the QUIC + tarpc transport, multiplexed channels, file transfer
  protocol, reconnect logic.
- `http/`: axum-based HTTP API.
- `vacuum.rs`: GC; runs a periodic daemon and a startup sweep for chunks
  left behind by crashed imports.
- `access.rs`: access-token file (mode 0o600 on Unix, never logged).

### `hub`

The matchmaker / discovery server. It:

- Accepts node connections, brokers content-hash queries via riddles so it
  never learns what content is being requested, just whether some peer
  claims to have a match.
- Forwards candidate responses back to the asker over a per-query
  `ChannelId`.
- Federates with other hubs via the hub-as-node protocol: a hub can act as
  if it were a node when talking to another hub.
- Records `ConnectionLog`, `QueryLog`, `CandidateLog`, `StatisticsLog` in
  LMDB, all keyed by a monotonic `Id` (microseconds since epoch, with
  `(last + 1).max(raw)` to defeat backward clock jumps).

Key modules:

- `rpc/{hub_server,hub_as_node,room,node_sampler}.rs`
- `models/`: the *Log structs plus `BlacklistedIp`.
- `replay_resistance.rs`: per-nonce deduplication. Read its module docstring
  before assuming it does more than that.

### `cli`

The `samizdat` command-line tool. It talks ONLY to the local node's HTTP
API at `http://127.0.0.1:<port>` with a bearer token from
`~/.samizdat/access-token`. It:

- Manages `Samizdat.toml` (public manifest) and `.Samizdat.priv` (secret
  manifest, written mode 0o600).
- Implements `init`, `import`, `commit`, `watch`, `series`, `subscription`,
  `hub`, etc.
- In `watch` mode, runs a loopback-only WebSocket server that pushes a
  `"refresh"` ping to the browser whenever a new edition is published.
  Origins on the upgrade are validated (loopback only).

### `proxy`

The public-facing piece. Runs on a server with a domain; terminates HTTPS
via ACME / Let's Encrypt; forwards GETs to a local node. Strips
`Authorization` and `Referer` headers, so the node sees every proxy-forwarded
request as a no-Referer GET that resolves to `granted=[Public]`. This is the
whole reason most node admin endpoints are safe to expose: the proxy simply
cannot reach them.

## Cross-cutting primitives

### Riddles

A `Riddle` is "find `h` such that `H(h || rand) == target`". The hub stores
`target` and `rand`; only a peer who knows `h` can prove it without revealing
`h`. The hash function (SHA3-224) is preimage-resistant, so the hub learns
nothing about which content a peer is looking up. `Hint` is an optional
prefix of `h` that bounds the solver's search space; longer hints reveal
more but make resolution faster.

`MessageRiddle` extends `Riddle` with an encrypted payload sealed using a
key derived from the solution: anyone who solves the riddle can decrypt the
payload (used to deliver candidate IPs back to the asker without the hub
learning them).

Used everywhere matchmaking happens: `Query.content_riddles`,
`EditionRequest.key_riddle`, `EditionAnnouncement.key_riddle`,
`Resolution.location_message_riddle`.

### Content addressing and `TransferCipher`

Every cipher in the wire protocol is keyed by a `Hash` of the content being
exchanged. `TransferCipher::new(content_hash, nonce)` derives a 32-byte AES-256
key via HKDF-SHA256 of `content_hash`. Two peers who agree on what they are
exchanging can encrypt/decrypt without negotiating a shared key. The AEAD
(GCM-SIV) is misuse-resistant, so reuse of `(content_hash, nonce)` does not
catastrophically leak.

This is also why the hub never sees content: every payload the hub forwards
is opaque-encrypted with a key the hub doesn't have.

### Bookmarks and refcounts

`Table::Bookmarks` stores per-object `(User, Reference)` counts as
`MergeOperation<i32>` entries. `User` is set/cleared by explicit user action.
`Reference` is incremented when an edition pins an object and decremented
when the edition is replaced. `Bookmark::is_marked` returns true if either
count is non-zero, which is how vacuum decides to spare an object.

Counts are `i32`; the previous design used `i16` and could wrap silently.
`saturating_add` in the merge operation keeps the count at `i32::MAX` rather
than wrapping if the (absurdly large) cap is ever reached.

`Bookmark::clear` deletes the entry unconditionally. `ObjectRef::drop_if_exists_with`
only clears `User`, never `Reference`, so a peer-driven advance that drops
an object cannot zero out the count of pinning editions. See the comment
near `drop_if_exists_with` for the full reasoning.

### Hub-as-node federation

The same tarpc service traits (`Hub`, `Node`) are used for both client-server
(`node -> hub`) and federation (`hub -> hub`). A `hub` configured with
`--partner-addr` opens a connection to another hub and acts as if it were a
node, allowing queries to fan out across the federation. The replay
resistance, throttling, and candidate channel mechanics work the same on
both paths.

### Transport: QUIC + multiplexed channels + tarpc

Each peer pair has a single QUIC connection. On top of QUIC, the code
multiplexes "channels": logical streams keyed by a 64-bit `ChannelId`. Each
channel carries either a tarpc RPC (length-delimited bincode frames) or
a file transfer stream. `node/src/system/transport/multiplexed.rs`
implements the channel layer; `Matcher` matches "expecting" and "arrived"
sides of a channel (the matcher is loose by design - it can drop matches if
a peer behaves badly, but no longer panics).

### Identity (Ethereum)

A series's public key can be associated with a human-readable name via a
smart contract on Polygon (chain ID 137). The contract maps
`name -> (entity, owner, ttl, data)`. The node queries Polygon via a
configurable RPC endpoint; the chain ID is verified on every read so an
operator who mis-configures the endpoint to a different chain gets an error
rather than a silently-wrong identity binding.

This is the only piece of the system that depends on a blockchain. Series
that don't need a human-readable name don't need it; the raw Ed25519 public
key works fine as an identifier on its own.

## Data flow walkthroughs

### Publishing

1. `samizdat init` creates `Samizdat.toml` (public) and `.Samizdat.priv`
   (mode 0o600) with a fresh Ed25519 keypair.
2. `samizdat commit` walks the build directory (refusing to follow
   symlinks), hashes each file into chunks, uploads them via
   `POST /_objects`, builds a `CollectionRef::build`, then calls
   `POST /_series-owners/<name>/editions` on the node.
3. The node signs the edition, persists it, and announces it to every
   connected hub. The hub broadcasts the announcement (still
   opaque-encrypted) to every connected node whose subscription riddle
   matches the series public key.

### Resolving

1. Browser requests `http://localhost:<port>/<series-name>/<path>`.
2. Node resolves `<series-name>` (either a raw pubkey or an identity-dapp
   lookup), looks up the latest edition, finds the locator hash for `<path>`.
3. If the object is locally cached, served directly. Otherwise:
4. Node creates a `Query` with `content_riddles` derived from the locator,
   sends it to every connected hub.
5. Hubs match the riddle against connected peers' subscriptions, ask
   matching peers via `Resolution`, get back candidates encrypted with a
   `MessageRiddle`.
6. Asking node decrypts candidates, opens QUIC connections, runs the
   file-transfer protocol from `node/src/system/transport/file_transfer/`.
7. Chunks arrive, are validated against the Merkle tree, written to LMDB.
   Vacuum will eventually GC them if no edition keeps the bookmark alive.

## Where to keep digging

- `system/transport/file_transfer/{mod,messages}.rs` for the chunk
  protocol.
- `system/transport/multiplexed.rs` for the channel layer.
- `rpc/{hub_server,hub_as_node,room}.rs` for the hub-side matchmaking.
- `models/series.rs::Edition` for the signed-pointer machinery (note the
  monotonicity check in `SeriesRef::advance`).
- `vacuum.rs` for the GC; `sweep_crash_leaked_chunks` for the startup
  recovery pass.
