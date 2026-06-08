# Upgrade hazards

What can desync between two versions of Samizdat in production, and where in
the code the desync is locked in. Only items verified from the codebase as of
0.3.2; theory and "what we should add some day" go to
[`deferred.md`](deferred.md).

## 1. CLI / daemon mismatch during `samizdat-up update`

The Linux and macOS update flow in
[`samizdat-up/src/install/linux.rs`](../samizdat-up/src/install/linux.rs)
and [`samizdat-up/src/install/macos.rs`](../samizdat-up/src/install/macos.rs)
follows the same order:

1. Replace every daemon binary on disk (`install_daemon_binary`).
2. Replace the `samizdat` CLI binary (`install_cli_binary`).
3. Restart each daemon (`systemctl restart` / `launchctl kickstart -k`).

Between step 2 and step 3, the new CLI is on disk while the daemon is still
running the old binary (Linux/macOS daemons keep executing the file mapped
into memory at start, even after the on-disk binary is replaced). If any
HTTP request shape changed between versions, a CLI invocation in that window
fails with a JSON deserialisation error on the daemon side. The 0.3.2
`series_owner_name` -> `nickname` rename is an example: an upgrade where the
user simultaneously runs `samizdat commit` from another shell will get
"missing field `nickname`" or "unknown field `series_owner_name`" from
whichever side is older.

Windows ([`install/windows.rs`](../samizdat-up/src/install/windows.rs)
`update()`) stops every daemon before replacing any binary, so there is no
running-old-daemon-with-new-CLI window; but during the stop-start interval
the CLI gets connection-refused instead.

A failed update that exits between step 2 and step 3 leaves the host in a
permanent skew until the operator re-runs `samizdat-up update` or
`systemctl restart samizdat-node`.

## 2. Bincode-serialised DB structs

Every `SeriesOwner`, `Hub`, `Edition`, etc. in
[`node/src/models/`](../node/src/models/) is `#[derive(Serialize, Deserialize)]`
and written via `bincode::serialize` to LMDB. Bincode is a positional format:

- Renaming a field is wire-safe (bincode does not encode field names).
- **Adding, removing, or reordering a field of any `Serialize/Deserialize`
  struct that ends up in an LMDB value is breaking.** Existing rows
  deserialise into junk or fail with "unexpected end of input".

The migration framework in
[`node/src/db/migrations.rs`](../node/src/db/migrations.rs) operates at the
table level only - it creates tables, repairs row contents, etc. There is no
mechanism for evolving the bincode layout of a value type; that has to be
authored by hand if a struct field is added (read old, decode with old
struct, write new).

The 0.3.2 `SeriesOwner.name` -> `SeriesOwner.nickname` rename was safe
specifically because bincode is positional; the same is NOT true of a future
addition of a field to the same struct.

## 3. tarpc + bincode RPC between hub and node, and between nodes

The Hub and Node services are declared at
[`common/src/rpc.rs:123`](../common/src/rpc.rs) and
[`common/src/rpc.rs:182`](../common/src/rpc.rs) via `#[tarpc::service]`. No
`protocol_version` RPC, no handshake exchange that compares versions; the
client trusts that the server speaks the same shape. Same bincode caveat as
above for any struct that crosses the wire (`Query`, `Resolution`,
`Candidate`, `EditionAnnouncement`, ...): renames safe, reorder/add/remove
break in-flight federation.

A hub running 0.3.x and a node running 0.4.x that changes any RPC arg
struct will lose federation silently (queries return empty, announcements
go nowhere) without surfacing the version skew explicitly.

## 4. `Samizdat.toml` schema has no version field

The manifest format
([`cli/src/manifest.rs`](../cli/src/manifest.rs),
template at [`cli/templates/Samizdat.toml.txt`](../cli/templates/Samizdat.toml.txt))
is plain serde-from-TOML, with no top-level `version` key and no migration
hook. Renaming or removing a key (which 0.3.2 did with
`name` -> `nickname`) makes every existing project's manifest fail to load
with "missing field" until the user manually edits it. Users with multiple
projects discover this one `samizdat commit` at a time.

## 5. Default daemon configs are written once and never overwritten

[`samizdat-up/defaults/node.toml`](../samizdat-up/defaults/node.toml) (and
the hub/proxy equivalents) are templated into `/etc/samizdat/<role>.toml`
by `ensure_config` in each platform install module. The file header says:

> samizdat-up will not overwrite this file on a reinstall.

Adding a required key to the default in a new version does not propagate to
existing installs. The daemon must either default the key in code (via
`serde(default)` or a fallback in `cli.rs`) or refuse to start, surfacing a
clear error. New keys that lack `serde(default)` and that the daemon
unconditionally reads will crash old installs on first restart after the
upgrade.

## 6. Identity-name DNS-safety filter is runtime-only

The smart contract was tightened to refuse DNS-unsafe identity names but
the on-chain bytecode is unchanged; the runtime
[`samizdat_common::identity::check_servable_identity`](../common/src/identity.rs)
is the only filter against pre-existing garbage until the deployment is
rotated. See [`blockchain/REDEPLOY.md`](../blockchain/REDEPLOY.md) for the
redeploy procedure.

## 7. Default-hub seeding is one-shot per install

[`samizdat-up/src/install/mod.rs::seed_default_hubs_best_effort`](../samizdat-up/src/install/mod.rs)
runs only during `samizdat-up install node`, not during `samizdat-up
update` or every node boot. Changing the contents of `DEFAULT_HUBS` in the
source has no effect on a host that has already gone through an
`install node`; the host keeps whatever hub list it has. New default hubs
have to either be added with an explicit migration step in `update()`, or
documented as "operators must run `samizdat hub new ...` manually".

## 8. Typed-subdomain dispatch replaces bare-key subdomains and admin reads

Content now lives at five prefix-labelled subdomain classes:
`object-<hash>.<root>`, `series-<key>.<root>`, `collection-<hash>.<root>`,
`edition-<id>.<root>`, and `<identity>.<root>` (no prefix). Existing
`<key>.<root>` series URLs and the node-side admin content read paths
(`GET /_objects/<hash>`, `GET /_collections/<hash>/<path>`) return 404;
the matching admin write paths (`POST /_objects`, etc.) stay. Bookmarks
and links pinned to either shape break and need to be reissued in the new
form. Cert and DNS are unchanged: one wildcard SAN, one wildcard A
record, with the type prefix inside the single wildcard label.

`Hash` and `Key` also gain a canonical RFC 4648 base32 lowercase
no-padding string form; the legacy base64-url encoding is dropped. The
on-chain identity entity strings stored on Polygon still carry the old
encoding for already-registered handles, so the resolver needs to accept
both base64 and base32 for one cycle. Tracked as a known migration knob;
see the identity resolver follow-up.

## 9. `/_kvstore/*` is gone

The node-side key-value store and its three routes (`GET`, `PUT`, `DELETE`
on `/_kvstore/{*tail}` and `DELETE /_kvstore/`) are no longer served. Pages
that called `sz.kvstore.{get,put,delete,clear}` now hit 404. Per-series
subdomain isolation makes the browser's own `localStorage`,
`sessionStorage`, and `IndexedDB` partitioned per origin, so authors get
the same key-value semantics without a round trip to the node; SamizdatJS
no longer exposes a `kvstore` property.

The `Table::KVStore` variant was also removed from the
[`Table`](../node/src/db/mod.rs) enum. LMDB sub-databases are addressed
by name (`Database::init` opens one handle per `Table::VARIANTS`
entry), so the on-disk sub-database named `KVStore` is simply no longer
opened on startup. Its bytes stay in the LMDB file as dead weight until
the node is wiped; `samizdat vacuum` does not reclaim them. Operators
upgrading an existing data directory should expect that residual size.
