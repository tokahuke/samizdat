# The cap model

How Samizdat decides whether to accept incoming object bytes. Read this
before touching `node/src/cap.rs`, `Edition::refresh`, vacuum, or the
import path in `node/src/models/object.rs`.

## The bookmark principle

The whole model rests on one observation:

> Anything that is not bookmarked can always be freed. Bookmarks cost
> because we commit to their persistence.

This is not a statement about *accounting* (every byte on disk counts
toward the disk cap, regardless of whether it is bookmarked -- the OS
sees the same disk usage either way). It is a statement about
*who can release the bytes*.

| Class | Vacuum's authority | When the cap fills with this class |
|---|---|---|
| **Bookmarked** | Vacuum **cannot** touch. The bookmark must be removed first by some other event (subscription advance, user drop, draft unmark). | Real "no more room for commitments." The operator must raise `max_storage`, unsubscribe, or drop user bookmarks. |
| **Non-bookmarked** | Vacuum can reclaim at any time. | Transient pressure. Vacuum runs, the cap drains, new imports proceed. |

A bookmark on an object (`Bookmark::mark(tx)` via `BookmarkType::User`
or `BookmarkType::Reference`) is a **persistent commitment**: the node
has promised some scope (a user, a subscription's edition, a future
pinner receipt) that the bytes stay on disk until that scope is
revoked. Vacuum respects that commitment.

A chunk or object that **no** bookmark points at is *unowned*. It used
disk for some reason (an on-demand HTTP resolve, a fetch whose
caller didn't bookmark, an orphan chunk from a failed import). The
disk cost is real *right now*, but vacuum will reclaim it on the next
sweep, so the operator's "long-term commitment" budget treats it as
transient buffer.

This dual nature is exactly the source of the two problems the cap
has to solve simultaneously:

1. **Storage must never exceed what the operator set.** This is a
   hard physical bound; vacuum runs on a schedule and cannot always
   react instantly. The accounting in `NODE_STORAGE_CAP` must
   therefore reflect *all* bytes on disk -- bookmarked plus
   non-bookmarked -- and reject imports that would push us over.
2. **Bookmarks are the real cost.** They reserve disk against
   vacuum's authority to free. A cap that is full of non-bookmarked
   bytes is recoverable; one that is full of bookmarked bytes is not.

So: the cap is a single hard ceiling on real disk usage; vacuum is
the release valve that drains *some* of it on demand; bookmarks
determine which portion is drainable.

## Three caps, three jobs

The `Cap` trait in `node/src/cap.rs` has four flavours (`Unbounded`,
`Budget`, `SizeLimit`, `Composite`) but only three load-bearing
instances in production code. They compose under `Composite` at the
single point of enforcement, `ObjectRef::import`:

```
let composite = Composite::new(vec![
    OBJECT_SIZE_LIMIT.clone(),  // ambient, stateless gate
    NODE_STORAGE_CAP.clone(),   // ambient, persistent budget
    caller_cap_or_unbounded,    // ephemeral per-refresh budget, or Unbounded
]);
let reservation = composite.reserve(supplied_metadata.content_size)?;
```

### 1. `OBJECT_SIZE_LIMIT` — `SizeLimit`, stateless

Bound: `cli().max_content_size * 1_000_000` (default 1 GB).

A single object's declared size cannot exceed this. Stateless: no
budget depletes; it is a gate. Defends against an honestly-published
1 TB object on the network.

Lives in: `cap::OBJECT_SIZE_LIMIT` static.

### 2. `NODE_STORAGE_CAP` — `Budget`, persistent

Bound: `cli().max_storage * 1_000_000` (default 1 GB).

The total bytes currently on disk in `Table::ObjectChunks` -- **all
of them**, bookmarked and non-bookmarked alike. This is the hard
ceiling: real disk usage cannot exceed it without an `ENOSPC`-style
failure in the rest of the node.

- Depletes on every successful import (`Reservation::commit` in
  `ObjectRef::do_import`).
- Restores on every vacuum drop of any object (`Cap::release` in
  `Droppable::drop_if_exists_with`), regardless of whether the
  object was bookmarked or unowned.

Lives in: `cap::NODE_STORAGE_CAP` static.

When the cap fills, the bookmark composition of the filler determines
how easily the node recovers:

- **Full of unowned bytes:** vacuum reclaims on its next sweep; new
  imports proceed. Tight but self-healing.
- **Full of bookmarked bytes:** vacuum can't help; new imports
  reject until an external event removes a bookmark (subscription
  advance clearing the previous edition's `Reference` bookmarks; user
  dropping `User` bookmarks; etc).

Reconstructed at startup from the on-disk truth -- see *Startup
reconstruction* below.

### 3. The per-refresh subscription cap -- `Budget`, ephemeral

Bound: `Subscription.max_bytes` (or
`cli().default_max_edition_size_mb * 1_000_000` if unset).

This is the cap a subscription's owner sets to say "I'm willing to
host up to N bytes of this series's current edition." It bounds **one
edition's inventory**, nothing larger and nothing longer-lived than a
single refresh.

`Edition::refresh` constructs a fresh `Budget` for this scope at the
start of every call (`cap::refresh_cap_for(public_key)`) and discards
it at the end. There is no cross-refresh state. Two refreshes for the
same subscription get independent budgets.

This is the key simplification:

- **No registry.** No `BTreeMap<Key, Budget>`, no `RwLock`, no
  startup reconstruction for subscription caps.
- **No edition diff.** When refresh advances E_old -> E_new, no
  release/reserve dance is needed; the new refresh has a new budget.
- **No subscription release path in vacuum.** Vacuum only releases
  `NODE_STORAGE_CAP`.

#### What the per-refresh cap enforces

During one `Edition::refresh` call, **every object in the new
edition's inventory reserves against the budget**, whether the bytes
need fetching or are already on disk. The reservation is what
defines "this edition's size against the cap." Fetched bytes and
already-local bytes both count because the cap measures **what this
edition touches**, not what this refresh downloads.

When a reservation fails the eager fetch for that item is skipped
(best-effort). Remaining items continue trying. The advance has
already committed by then, so the subscription stays at the new
edition; the missing items get fetched on-demand later, or stay
missing if they remain over budget.

#### What the per-refresh cap does NOT enforce

- It does **not** enforce that the *combined* on-disk size of E_old +
  E_new during a refresh stays under the cap. That overlap is brief
  and is the job of `NODE_STORAGE_CAP` (which sees real disk
  pressure), not the per-edition budget.
- It does **not** track the subscription's pinned bytes across
  refreshes. Persistence is a `NODE_STORAGE_CAP` property, mediated
  by bookmarks (`BookmarkType::Reference` is what
  `SeriesRef::advance` mark/unmarks).
- It does **not** apply to on-demand HTTP reads. Those pass `None`
  as the caller cap; only `OBJECT_SIZE_LIMIT` and `NODE_STORAGE_CAP`
  gate them.

## Reservation lifecycle

`Reservation` is RAII. The default behaviour on Drop is to call
`Cap::release`, restoring the bytes. `commit()` marks it permanent --
the budget stays decremented until an explicit `Cap::release` call
(vacuum) returns it.

| Outcome | What happens |
|---|---|
| Import succeeds | `do_import` calls `reservation.commit()` inside the same `writable_tx` that records `ObjectStatistics`. Budget stays decremented. |
| Import fails (network, hash mismatch, exact-size mismatch, anything) | Reservation drops out of scope. RAII releases all budgets back to their full value. No accounting drift. |
| Object later deleted by vacuum | Vacuum reads the object's `content_size`, calls `NODE_STORAGE_CAP.release(size)`. (Per-refresh budgets are long gone; nothing else to release.) |

The atomicity of the composite reserve means a partial success
(reserved 2 of 3 child caps) rolls back via the same RAII path
before returning the error.

**`commit()` is keyed to disk presence, not to bookmark presence.**
The moment `create_object_with` writes chunks to LMDB inside the
`writable_tx`, the bytes are on disk and should count against
`NODE_STORAGE_CAP`. `commit()` runs in that same transaction, so
budget and disk move atomically. Whether the object also gets a
bookmark (refresh's path) or stays unowned (HTTP resolver's path) is
an independent decision made by the caller after import returns. The
cap only tracks "bytes on disk"; the bookmark layer tracks "who
promised to keep them."

## Startup reconstruction

`NODE_STORAGE_CAP` is the only cap that needs to be reconstructed at
boot. Walk `Table::ObjectStatistics` summing `content_size` over
**every** persisted object -- bookmarked and unowned alike -- because
the cap reflects real disk usage. Subtract the sum from
`max_storage * 1_000_000` to seed `NODE_STORAGE_CAP.remaining`.

Per-refresh budgets need no reconstruction; they are created on
demand each time a subscription's `refresh()` runs.

`OBJECT_SIZE_LIMIT` is stateless; nothing to seed.

(Unowned bytes that survive across a restart get reclaimed by
vacuum on the first sweep after boot, which restores their portion
of the budget. The initial conservative count makes sure we never
*understate* current disk pressure during the boot window.)

## What it doesn't do (deferred)

Three things are intentionally out of scope for V1; flagged here so a
future contributor doesn't add them silently:

1. **Per-publisher caps.** When pinner receipts (see
   `docs/rfc-pin-receipts.md`) land, the receipt's `max_bytes` will
   want to compose with the per-refresh cap as another scope. The
   `Cap` trait + `Composite` already accommodate this; the only
   change is adding the receipt's `Budget` to the caller's vector.
2. **Cap usage in the subscription API.** `GET
   /_subscriptions/{key}` could report current usage. Today the
   per-refresh budget is ephemeral so "current usage" only exists
   *during* a refresh. To expose stable usage, query
   `Table::ObjectStatistics` filtered by the subscription's last
   edition's inventory. Useful but not load-bearing.
3. **`loom`-level concurrency tests on `Composite`.** The
   atomic-budget rollback is straightforward, but a model checker
   would prove the invariant under all interleavings. Pure
   nice-to-have.

## Common questions

**Q. If subscription caps are per-refresh, what stops a subscription
from accumulating more than its `max_bytes` over time?**

A. The bookmark machinery. `SeriesRef::advance` clears the previous
edition's `Reference` bookmarks and places new ones. An object that
falls out of the inventory loses its `Reference` bookmark refcount;
once it hits zero (no other bookmark type holds it) vacuum is free
to reclaim. So a subscription's persistent footprint is bounded by
"the size of its current edition's inventory" -- which is exactly
what the per-refresh cap enforced when that edition arrived.

**Q. What if vacuum is slow and the brief E_old + E_new overlap
genuinely exceeds `max_storage`?**

A. `NODE_STORAGE_CAP` will reject the new fetch (it sees real disk
usage). The refresh leaves the offending object(s) unfetched; vacuum
drains the now-unmarked E_old objects on its next sweep; the next
refresh attempt (or on-demand fetch) succeeds. This is the right
back-pressure behaviour at the storage layer.

The same back-pressure protects against a node that is **already
over its bookmarked cap** at boot (e.g. operator lowered
`max_storage` between runs): startup reconstruction sees the
overflow, every reserve fails until vacuum + subscription advances
release enough bookmarks. The node serves what it has but refuses
new commitments.

**Q. Why not also have a series-level or collection-level cap?**

A. They collapse to the per-edition cap in practice. A series's
pinned bytes at any moment are exactly its current edition's
inventory; a collection IS an edition's content snapshot. The
existing cap is the natural granularity.

**Q. What happens on a `GET /<hash>` HTTP read for an object the
node doesn't have locally?**

A. The resolver (`node/src/http/resolvers.rs`) calls
`hubs().query(...)` and receives a `FetchOutcome::InFlight`. It then
calls `ObjectRef::import` with `caller_cap = None`. The composite is
`[OBJECT_SIZE_LIMIT, NODE_STORAGE_CAP, Unbounded]`:

- `OBJECT_SIZE_LIMIT` rejects an oversized peer-declared object
  before any chunk arrives.
- `NODE_STORAGE_CAP` rejects if the disk budget is already full.
- The third slot is `Unbounded` because the resolver fetch is not
  bound to any subscription's per-edition budget; this is an
  on-demand serve, not a pinning event.

On success, the object lands on disk and is **unowned** -- the
resolver does not bookmark it. The bytes count against
`NODE_STORAGE_CAP` for as long as they exist, but vacuum is free to
reclaim them on the next sweep. The reservation is `commit()`ed
because the bytes are real; the eventual `Cap::release` fires from
`Droppable::drop_if_exists_with` when vacuum drops the unowned
object.

In short: HTTP reads get to use the disk cap as a transient buffer.
They cannot exceed it, but they also cannot pin against it.

**Q. What about a fetch the user explicitly bookmarked with
`samizdat object new`?**

A. Same import path; same reservation. The difference is in what the
caller does *after* `import` returns: a `BookmarkType::User` mark
gets placed (`bookmark = true` in `do_import`'s call to
`create_object_with`), and vacuum can no longer touch the bytes
until the user explicitly drops them. The budget decrement persists
exactly until that drop happens.
