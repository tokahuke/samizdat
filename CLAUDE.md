# Agent orientation

Samizdat is a peer-to-peer, content-addressed publishing network with Ethereum-based
identity. The workspace has five Rust crates: `common`, `node`, `hub`, `cli`, `proxy`.
This file tells you (the agent) where to look first.

## Read these before doing anything substantial

1. **[docs/architecture.md](docs/architecture.md)** - what each crate does, the key
   concepts (riddles, series/editions/collections/objects, hub federation), how
   content flows from publisher to consumer. Read this first.
2. **[docs/threat-model.md](docs/threat-model.md)** - who can talk to whom over
   which transport, what authentication exists at each boundary, and what an
   attacker on each surface can do. Read this before flagging any "security" bug;
   most of them have non-obvious saving graces from the larger model.
3. **[docs/audit-history.md](docs/audit-history.md)** - the bugs that were found
   and fixed in the multi-pass audit, plus the deferred items with their
   rationale. Do NOT re-flag anything on the deferred list without reading why
   it was deferred.
4. **[docs/conventions.md](docs/conventions.md)** - DB layer usage, test harness,
   error handling, coding patterns the rest of the codebase already follows.
5. **[docs/extras.md](docs/extras.md)** and **[docs/operations.md](docs/operations.md)** -
   the non-Rust pieces (js/, install/, blockchain/, simulate-net/, terraform/,
   .github/workflows/) and the runbook for operating the public testbed
   (`testbed.hubfederation.com`). Read these before touching anything
   release-pipeline or deploy-shaped.
6. **[docs/deferred.md](docs/deferred.md)** - the actionable backlog across all
   audit passes. Before opening a "should we also..." conversation, check
   whether it's already there with the rationale.
7. **[.cursorrules](.cursorrules)** - file-level code style (indentation, naming,
   docstring rules). The conventions doc complements it; do not duplicate.

## Working preferences (Pedro's, applied always)

These live in `~/.claude/projects/.../memory/feedback_preferences.md` for the
human's agent. The TL;DR for agents that don't have access to that memory:

- Docstrings and comments wrap to at most 90 columns; ASCII-only punctuation
  (no em-dash, en-dash, ellipsis, smart quotes, arrows). Source code goes up
  to 100 columns (see `.cursorrules`).
- Avoid nested `use` statements inside function bodies. `cfg`-gated imports
  (e.g. `#[cfg(unix)] use std::os::unix::...`) are fine where they appear.
- Do not run `cargo test` after every edit. Batch related changes and validate
  at a sensible breakpoint. The full suite is slow; one run at the end of a
  feature is enough.
- Commits are one-liners. No multi-line bodies, no "Co-Authored-By" trailers,
  no AI attribution. Pedro is the sole author.

## How the workspace builds

- Toolchain: nightly Rust (one `#![feature(try_blocks)]` in `node/src/main.rs`
  is the only remaining nightly dependency). Everything else compiles on
  stable.
- Single workspace at the repo root; `cargo check --workspace` and
  `cargo test --workspace` work. Test count as of the last sweep: 51 unit
  tests across `common`, `hub`, and `node`.
- DB tests use the `samizdat-common` `test-helpers` feature. See
  `docs/conventions.md` for the `TestDb<T>` pattern.

## Conventions that are easy to miss

- The DB error model (LMDB) was deliberately rewritten so EVERY accessor on
  `Table`, `TableRange`, `TablePrefix`, `Migration` returns `Result`. No
  `panic!`/`expect()` on a DB error anywhere in production code. New code
  must follow the same pattern.
- `ChannelId` is u64 (was u32 before the audit). Random allocation in
  `ChannelId::random()`. `ChannelAddr::from_socket_and_hash` reads 8 bytes
  of the hash; never assume the type is `u32`.
- `PrivateKey: Debug + Display` is **redacted on purpose**. To serialise the
  secret (for persistence to disk), call `PrivateKey::reveal_base64()`
  explicitly. Anywhere a debug print would otherwise expose the secret, the
  redaction will mask it; do not "fix" this by adding a Display impl that
  shows the bytes.
- Replay resistance lives in `hub/src/replay_resistance.rs`. It does NOT
  validate timestamps (no signed timestamp in the messages); the docstring
  there explains the actual security guarantee. Do not add a fake
  `now - then > MAX_AGE` check; it would be a no-op against any motivated
  attacker.

## When in doubt

- Read `docs/threat-model.md` before flagging anything as a security bug.
- Read `docs/audit-history.md` before flagging anything that "looks scary".
- The B-series findings (B1-B15) and N-series findings (N1-N6) and H-series
  (H1-H13) are catalogued by short tag in the audit history; an agent that
  re-discovers one should reference the existing tag rather than re-naming
  it.
