# Coding conventions

Style basics live in `.cursorrules`; this doc covers cross-cutting patterns
the codebase relies on. New code should follow these without explanation.

## Style summary (with `.cursorrules` as source of truth)

- 100 columns for source code, 90 columns for docstrings and comments.
- snake_case variables/functions/files; camelCase types; UPPER_CASE constants.
- `mod` declarations come before `use`.
- ASCII-only punctuation in comments and docstrings. No em-dash, en-dash,
  ellipsis, smart quotes, arrows. Use `;` or `.` instead of em-dash;
  `->` instead of `->`.
- Avoid nested `use` statements inside function bodies. `cfg`-gated imports
  for a specific platform (e.g. `#[cfg(unix)] use std::os::unix::...`) are
  fine in place because moving them out forces a bigger cfg block. Prefer
  a top-of-file `use std::path::PathBuf` over fully-qualified
  `std::path::PathBuf` inline in signatures.
- Documentation precedes macro directives.

## Errors

The workspace has a single `Error` type at `samizdat_common::Error`, with
`From` impls for the usual stdlib and dep errors (`io::Error`, `bincode`,
`base64`, `quinn::ConnectionError`, etc.) and a `From<String>` /
`From<&'static str>` so you can `.into()` an ad-hoc message. The enum is
`#[non_exhaustive]`; adding variants does not break clients.

Don't `panic!` on conditions reachable from a peer or user. Don't `expect()`
on a Result whose Err type is a real failure mode (`I/O`, `bincode`, an
LMDB error). Use the question mark.

`anyhow::Error` is used in `cli` and `proxy` because those binaries are
the user-facing surface; in `node`, `hub`, and `common` use
`samizdat_common::Error` directly so the type alignment is obvious.

## DB layer

The LMDB wrapper in `common/src/db.rs` is the single way to talk to the
database. Two transaction types, `WritableTx` and `ReadonlyTx`, paired
with the closures `writable_tx(...)` and `readonly_tx(...)`. Inside the
closure you call methods on the per-crate `Table` enum.

### Every accessor returns `Result`

```rust
// Read.
let maybe_value: Option<Vec<u8>> = readonly_tx(|tx| {
    Table::Foo.get(tx, key, |bytes| Ok(bytes.to_vec()))
})?;

// Read and decode (closure returns Result, propagates).
let decoded: Option<MyType> = readonly_tx(|tx| {
    Table::Foo.get(tx, key, |bytes| Ok(bincode::deserialize(bytes)?))
})?;

// Write.
writable_tx(|tx| {
    Table::Foo.put(tx, key, value)?;
    Ok(())
})?;
```

The closure passed to `get` must return `Result<T, crate::Error>`. If the
closure body never fails (e.g., a vec copy), still wrap in `Ok(...)`. The
question mark inside the closure propagates into the outer `Result<Option<T>, Error>`
that `get` returns.

### Range and prefix iteration

```rust
// Returns Result<Option<U>, Error>. Ok(Some(value)) breaks early.
let found = readonly_tx(|tx| {
    Table::Foo.range::<_, [u8; 0]>(..).for_each(tx, |key, value| {
        if matches(key) {
            Ok::<Option<MyType>, crate::Error>(Some(bincode::deserialize(value)?))
        } else {
            Ok(None)
        }
    })
})?;

// Returns Result<C: FromIterator<V>, Error>. Closure is infallible.
let all: Vec<MyType> = readonly_tx(|tx| {
    Table::Foo.range::<_, [u8; 0]>(..).collect(tx, |_, value| {
        bincode::deserialize::<MyType>(value).expect("encoded by us")
    })
})?;
```

If you need a fallible closure in `collect`, return `Result<T, E>` from
the closure and collect into `Result<Vec<T>, E>`; the outer
`Result<Result<Vec<T>, E>, db::Error>` flattens with two `?`s.

### Never `panic!` on a DB error

The old code panicked on every LMDB error other than `NotFound`. The new
code propagates. If a function downstream wants `Ok(default)` on a
specific error, it does that itself; the layer below does not pre-decide.

## Test harness for DB

`samizdat-common` exposes a `test-helpers` feature that turns on
`db::test_harness::TestDb<T>`. In a downstream crate's `[dev-dependencies]`:

```toml
[dev-dependencies]
samizdat-common = { path = "../common", features = ["test-helpers"] }
tempfile = "3.12"
```

Then:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use samizdat_common::db::test_harness::TestDb;

    #[test]
    fn my_test() {
        TestDb::<crate::db::Table>::with(|| {
            // writable_tx and readonly_tx work as in production.
            let id = unique_name("foo");  // see below
            writable_tx(|tx| {
                Table::Foo.put(tx, id.as_bytes(), b"value")?;
                Ok(())
            }).unwrap();
        });
    }
}
```

Tests in the SAME binary share the global LMDB singleton (the `OnceLock` in
`init_db`), so the FIRST `TestDb::new()` initializes on a fresh tempdir and
subsequent calls reuse that handle. Each test holds a `MutexGuard` for the
duration of the closure, so tests run serially within a binary. Keys leak
across tests; use unique names (e.g. `format!("{}-{}", base, Hash::rand())`)
so co-existing rows don't interfere.

If you need a TRULY clean slate per test, split the suite across multiple
`tests/*.rs` integration binaries; each binary has its own singleton.

## Channel ids

`samizdat_common::address::ChannelId` is a `u64` newtype. Constructors:

- `ChannelId::random()` allocates a fresh cryptographically random id.
- `ChannelId::from_be_bytes([u8; 8])` parses the wire representation.
- `id.to_be_bytes()` serialises.

Never assume the underlying type. Don't `u32::from(...)`; don't pass `u32`
on the wire.

## Private keys and secret material

- `PrivateKey: Debug + Display` are redacted. To serialise the key (to disk
  or for explicit display to the user), call `reveal_base64()`. The user
  should see this method name and understand they're touching a secret.
- `PrivateKey` is `Zeroize + ZeroizeOnDrop`.
- Files containing secret material (`.Samizdat.priv`, access tokens, etc.)
  are written with mode 0o600 on Unix via the helper pattern used in
  `cli/src/manifest.rs::write_priv_file` and `node/src/access.rs`.
- HTTP responses from the local node that could carry secret material
  (e.g. `/_series-owners*`) are redacted in CLI logs via
  `cli/src/api/mod.rs::redact_if_sensitive`. Add new sensitive routes to
  `SENSITIVE_BODY_ROUTES` rather than redacting at each call site.

## Logging

`tracing` everywhere. The default level is `info` in production binaries.
Things to keep in mind:

- Don't log secrets. Use redaction helpers; demote to `debug!` if you must
  log something potentially sensitive (e.g. operator email in proxy CLI
  args).
- Don't log full HTTP response bodies at `info`. The CLI has
  `redact_if_sensitive` for this; use it for new endpoints if there's any
  chance the body is sensitive.
- For RPC handlers, prefer `tracing::error!` + return `InternalError` (or
  equivalent) over `panic!`. The hub and node both have many surfaces
  where a panic kills a connection task and silently disconnects a peer.

## Concurrency

- Use `tokio::sync::Mutex` for cross-await locks; `std::sync::Mutex` or
  `parking_lot::Mutex` only for short critical sections that never
  `.await`.
- Don't hold a `Mutex` across a DB transaction. LMDB serialises writers on
  its own; an outer mutex just compounds contention. The replay-resistance
  rewrite is the canonical fix example: see `hub/src/replay_resistance.rs`.
- Channels: prefer bounded `mpsc::channel` over `unbounded`. The
  `KeyedChannel` in `common` is bounded by `CHANNEL_CAPACITY = 1024`; copy
  that pattern.
- For "spawn a cleanup task on each call" patterns, consider whether a
  single `DelayQueue` would do; the matcher is the canonical TODO example.

## File system

- Refuse to follow symlinks where the bytes will be uploaded or otherwise
  exfiltrated. Use `fs::symlink_metadata` and check `file_type().is_symlink()`.
  See `cli/src/commands/mod.rs::commit::walk` for the pattern.
- Write secret files with mode 0o600 (Unix). See the
  `cli/src/manifest.rs::write_priv_file` pattern.
- Don't `expect()` on `to_str()` of a path. Non-UTF-8 paths are legal on
  Unix.

## HTTP responses (node)

- Authenticated routes go in the appropriate `nest("/_thing", ...)` block
  in `node/src/http/mod.rs::api`. Mutating routes need a `security_scope!(...)`
  middleware with the appropriate `AccessRight`.
- Routes that should only be callable by the operator (e.g. `/_vacuum/*`)
  use `authenticate_trusted_context` middleware: bearer OR `/_register`
  trusted context.
- For routes that may be reached via the proxy (any GET with `Public`
  scope), remember that no `Authorization` or `Referer` headers will be
  present from the client.

## Adding a new RPC method

`Hub` and `Node` services are defined in `common/src/rpc.rs` via
`#[tarpc::service]`. To add a method:

1. Add the method to the trait and its request/response types.
2. Implement on `HubServer` (`hub/src/rpc/hub_server.rs`), on
   `HubAsNodeServer` if hub-to-hub federation needs it
   (`hub/src/rpc/hub_as_node.rs`), and on `NodeServer`
   (`node/src/system/node_server.rs`).
3. If the input message can be replayed, implement `Nonce` for it in
   `hub/src/replay_resistance.rs` and call `REPLAY_RESISTANCE.check(&msg)?`
   in the handler.
4. If the method does heavy work, gate it via the per-connection
   `throttle` helper.
5. Wire-format-breaking changes require careful coordination since the
   workspace ships node + hub together; pre-launch this is free.

## Adding a new LMDB table

1. Add the variant to the appropriate `Table` enum
   (`node/src/db/mod.rs` or `hub/src/db/mod.rs`); update the variant
   array used by `strum::VariantArray`.
2. Add a migration in `migrations.rs`. The migration body can be a no-op
   for a new empty table; LMDB creates the database lazily on first use.
3. Document the value schema where the table variant is defined.

## Vacuum and refcounts

If your code references an object, it should `mark` the
`BookmarkType::Reference` bookmark when the reference is created and
`unmark` it when the reference is removed. The vacuum will spare any
object whose `Reference` or `User` count is non-zero.

If you write chunks outside `do_import` (e.g. a future "import from
trusted source" path) you must also bump `ObjectChunkRefCount`; otherwise
the chunks are vacuum-eligible the moment they exist. The startup sweep
`vacuum::sweep_crash_leaked_chunks` only knows about the "no refcount at
all" case.

## When to ask the user

This is a working preference (see `~/.claude/projects/.../memory/feedback_preferences.md`),
but tactically: don't ask before applying mechanical fixes that you have a
plan for. DO ask before:

- Wire-format-breaking changes that aren't part of an in-progress refactor.
- Adding new dependencies.
- Anything that destroys data (DROP TABLE-equivalent, force-pushes, etc.).

Pre-launch, treat the codebase as malleable; bias toward "fix it and
explain in the commit message" over "ask permission".
