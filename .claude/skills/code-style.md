---
name: samizdat-code-style
description: Use when writing, reviewing, or refactoring Rust code in the samizdat workspace. Covers imports, docstring width, comment discipline, banner smells, and version-narration rules. Read this before non-trivial edits so the style doesn't drift; the rules below are stricter than rustfmt's defaults and stricter than what most agents do by default.
---

# Samizdat code style

The non-obvious rules. Rustfmt and clippy handle the mechanical ones; this
file is for the project-specific judgement calls that get re-litigated
every PR if they aren't written down. Pedro is the sole author and will
reject changes that violate these. Read before editing.

The bedrock file is `.cursorrules` at the repo root (snake_case for
variables, camelCase for types, 4 spaces, 100-col code, 90-col
docstrings, every public item documented). This file *supplements* it
with the things that have actually tripped agents in practice. When this
file and `.cursorrules` agree, `.cursorrules` wins; when they disagree,
this file is the more recent thinking.

## Imports

### Group order

Three groups, separated by **one** blank line:

1. External crates (alphabetical).
2. `samizdat_common::*` (alphabetical).
3. `crate::*`, then `super::*`.

Within each group, alphabetical sort. Example (from `node/src/vacuum.rs`):

```rust
use ordered_float::NotNan;
use samizdat_common::db::{readonly_tx, writable_tx, Droppable, Table as _, WritableTx};
use serde_derive::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::time::{sleep, Instant};

use samizdat_common::heap_entry::HeapEntry;
use samizdat_common::Hash;

use crate::cli::cli;
use crate::db::{MergeOperation, Table};
use crate::models::{CollectionItem, ObjectMetadata, ObjectRef, ObjectStatistics, UsePrior};
```

**Important:** `cargo fmt` cannot enforce the group split. Rustfmt
sorts imports within whatever blank-line-delimited chunk it finds, but
it does not know that `samizdat_common` is a "workspace" crate that
should be its own group. So `samizdat_common::*` imports can end up
mixed with externals if a previous edit put them in the same chunk.
Always hand-organize: external in chunk 1, all `samizdat_common::*`
in chunk 2, all `crate::*` in chunk 3, `super::*` in chunk 4. After
that, `cargo fmt` will keep them stable.

The `rustfmt.toml` at the repo root does NOT set `group_imports`,
specifically because rustfmt's "StdExternalCrate" grouping would put
`samizdat_common::*` in the external group. Leave that setting
unset.

What rustfmt DOES enforce (via `imports_granularity = "Crate"` in
`rustfmt.toml`) is collapsing multiple `use` statements from the same
crate root into one nested `use`. So `use std::sync::Arc;` plus
`use std::time::Duration;` plus `use std::fmt::Display;` becomes:

```rust
use std::{fmt::Display, sync::Arc, time::Duration};
```

This applies to `std`, external crates, `samizdat_common`, and
`crate::*` -- all imports rooted at the same crate get collapsed into
one statement per group. You don't need to write the nested form
yourself; `cargo fmt` will do it.

### `{}` syntax: USE IT

When two or more items come from the same crate root, use Rust's
`{}` syntax. `cargo fmt` enforces this through `imports_granularity =
"Crate"` in `rustfmt.toml`, so you mostly don't have to think about
it -- write your imports however and `cargo fmt` will collapse them.

```rust
// What you can write:
use crate::cap;
use crate::cap::Cap;
use crate::cli;
use crate::hubs;

// What cargo fmt produces and what gets committed:
use crate::{
    cap::{self, Cap},
    cli,
    hubs,
};
```

`cargo fmt` will not move imports across blank-line-separated groups,
so you still need to hand-organize the groups (external /
samizdat_common / crate / super) before running it.

### No `use` inside function bodies

Never write `use foo::bar;` inside a `fn`. Lift it to module scope or
use a fully-qualified path call (`foo::bar(...)`). `cfg`-gated imports
(`#[cfg(unix)] use std::os::unix::...`) are fine where they appear.

## Docstrings

### Width: 90 columns

`///` and `//!` lines wrap at 90 columns total (including the
indentation and the `///` prefix). Source code goes up to 100; only
docstrings are stricter. The 90-col target is for human readability in
side-by-side diff viewers and on narrow terminals.

### Placement

- Docstrings come **above** any macro attributes (`#[derive(...)]`,
  `#[serde(...)]`, etc.). The attribute is part of the item; the
  docstring describes the item, so it leads.
- Every public item gets a docstring (functions, types, traits, fields,
  consts, statics). Private items too, with one exception below.
- Module-level documentation lives at the top of the module file using
  `//!`. Do not document a `mod foo;` declaration in another file -- the
  documentation goes in `foo.rs` / `foo/mod.rs`.

### Two exceptions

- Items inside a `impl Trait for Type` block do **not** need docstrings
  (the trait itself documents the contract).
- Nested functions and nested structs do not need docstrings.

### Tone

Clear and succinct. Lead with what the item is, then **why** if the why
is non-obvious. Don't restate the signature. Don't write
"This function takes X and returns Y" -- the reader can see that.

## Comments

### No bibles in code

If a comment is longer than ~3 short lines or more than one paragraph,
it does not belong in code. Move it to a doc under `docs/` (or
`docs/upgrade-hazards.md`, or `docs/cap-model.md`, etc.) and reference
the doc from the code with a one-line pointer.

Bad (committed by an agent and rejected):

```rust
// 3. Eager fetch with per-edition cap enforcement. Best-effort:
//    the advance has already committed (step 2), so any
//    individual fetch failure or cap rejection just leaves
//    that item unfetched. The parallel fan-out lets fetches
//    that fit run alongside ones that don't; FuturesUnordered
//    collects outcomes as they complete.
// ... 14 more lines ...
```

Good:

```rust
// 3. Eager fetch under per-edition cap. Best-effort: failures
//    leave items unfetched.
```

The full story lives in `docs/cap-model.md`. The code points there if a
reader needs more; otherwise the inline `//` says only what the reader
needs to make sense of the **next** few lines.

### No "WHAT" comments, only "WHY"

Don't explain what the code does -- well-named identifiers do that.
Comments are for surprises:

- A hidden constraint the reader can't infer from the code.
- A subtle invariant that's load-bearing but not enforced by types.
- A workaround for a specific bug or quirk.
- Behavior that would surprise a thoughtful reader.

If removing the comment wouldn't confuse a reader who knows Rust and
samizdat, the comment shouldn't exist.

### No version or migration narration

Never write comments like:

- `// 0.3.3: renamed from X to Y`
- `// pre-0.4 this returned Option<T>`
- `// in this migration we...`
- `// added for the Y flow`
- `// used by Z`

That information lives in `git log`, the PR description, or
`docs/upgrade-hazards.md`. In code it rots; the rename name stays even
after the version it referenced is forgotten.

## Banners

`// ============================================` separator banners
inside a module are a smell. They usually mean the module wants to
split. If you reach for one, ask whether the section belongs in a
sub-module; if it doesn't, just let the doc-comments on the items be
the structure.

## Other things to remember

- **Commits are one-liners.** No multi-line bodies, no "Co-Authored-By"
  trailers, no AI attribution. Pedro is the sole author.
- **ASCII-only punctuation in code, comments, and docstrings.** No
  em-dash, en-dash, ellipsis, smart quotes, arrows. Use `--` or `-`,
  `...`, `'`, `"`, `->` instead.
- **Tests are slow.** Do not run `cargo test` after every edit. Batch
  related changes and validate once at a sensible breakpoint.
- **PR descriptions are minimal.** Short Summary, Breaking changes,
  Version. No "Test plan" checklist. No exhaustive bullet lists.
- **Error handling.** The DB layer was deliberately rewritten so every
  accessor returns `Result`. No `panic!`/`expect()` on DB errors in
  production code. Use `?` and propagate `crate::Error`.

## When in doubt

- Style: read `.cursorrules` first, then this file. They are intended
  to be consistent; if you find a conflict, this file is more recent.
- Code patterns: read `docs/conventions.md` for the DB layer, test
  harness, and patterns the rest of the codebase already follows.
- Architecture: read `docs/architecture.md` and the top-level
  `CLAUDE.md` for orientation.
