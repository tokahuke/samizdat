# RFC: pin receipts

## Status

draft. Targeted release: pinner V1.5 (replaces the V1 shared `X-Api-Key`
auth before the pinner has any real customers).

## Motivation

Pinner V1 ships with a single shared `api_key`: every customer uses the
same key, the pinner cannot attribute pins to who paid, and there is no
way to delegate pin management without sharing the publisher's series
private key. The V1 model assumed "one operator, one trusted customer"
and breaks the moment a second customer arrives.

The natural fix is "pubkey-attributed pins," but the first cut of that
design (a `payer_pubkey` column plus an `allowed_payers` allowlist) ran
into a tighter version of the same problem: who is the payer if a
donor pays for someone else's series? what if the publisher loses their
key but supporters want to keep the content online? what if a friend
wants to handle pinning ops without holding the publisher's signing
key? "Payer" turned out to be a poor abstraction because it conflated
*who can authorize a pin*, *who paid for it*, and *who can manage it
later*. Those are three distinct concerns and the design should reflect
that.

## Design: pinner-issued, pubkey-bound bearer credentials

When a pin is created the pinner issues a signed receipt:

```rust
struct PinReceipt {
    series_key: Key,
    expires_at: DateTime<Utc>,
    /// The pubkey authorised to renew or drop this pin. Any keypair
    /// the requester chooses at issuance time; the pinner does not
    /// care whose key this is.
    bound_to: Key,
    /// Random per-receipt; lets the pinner detect replay across
    /// receipts that are otherwise identical and gives an audit
    /// handle.
    receipt_id: Hash,
    /// ed25519 signature by the pinner's own keypair over the four
    /// fields above.
    pinner_sig: Signature,
}
```

The receipt is verifiable by *anyone* (the pinner's pubkey is operator-
public). The receipt is *redeemable* only by whoever holds the private
key matching `bound_to`. Renew and drop carry the receipt plus a
signature by `bound_to` over a request envelope including a fresh nonce.

The pinner verifies four things on every redemption:

1. `pinner_sig` is a valid ed25519 signature by the pinner's pubkey
   over the canonical bincode of (`series_key`, `expires_at`,
   `bound_to`, `receipt_id`).
2. The request envelope is signed by `bound_to`.
3. The envelope's nonce has not been seen within the receipt's
   replay-resistance window.
4. The current row in LMDB still references this `receipt_id` (a
   superseded receipt cannot redeem a slot that has been renewed
   under a new receipt).

## Why this collapses every authorization-shaped question to one mechanism

| Use case | Payer | `bound_to` | Notes |
|---|---|---|---|
| Self-publishing | publisher | series's own pubkey | the simple case |
| Sponsor pays for someone's series | donor | series pubkey (gift) | donor walks away |
| Friend manages publisher's pinning | publisher | friend's pubkey | no series-key sharing |
| Anonymous donor, donor stays in control | donor | donor's pubkey | unrelated to series owner |
| Dead-author preservation | author | trusted community pubkey | pre-issued before death |
| Multi-pinner redundancy | publisher | series pubkey (each pinner) | N independent receipts |

The series owner never has to be online at redemption. The pinner never
has to know who paid. The `bound_to` is a publisher-controlled lever,
not an operator-controlled allowlist.

## The schema that drops out

LMDB row keyed by `series_key`:

```
PinnedRow {
  expires_at: DateTime<Utc>,
  receipt_id: Hash,
  bound_to: Key,
}
```

The `customer: Option<String>` field from V1 disappears. Same for the
`payer_pubkey` column the V1.5-first-draft was going to add.

## HTTP surface

```
POST /pin
body: { series_key, days, bound_to, payment_proof? }
returns: PinReceipt (signed)

POST /renew
body: { receipt: PinReceipt, days, nonce, sig_by_bound_to }
returns: PinReceipt (signed, new expiry)

DELETE /pin
body: { receipt: PinReceipt, nonce, sig_by_bound_to }
returns: 204 NoContent

GET /pin/{series_key}
returns: { expires_at, bound_to }   // public; anyone can query state
```

`payment_proof` is the orthogonal funding axis: empty for the mutual-
aid model, a signed operator credit for the off-band-paid model, or
an on-chain receipt for the future chain-watched model. The receipt
schema does not change.

## Migration from V1

The V1 `X-Api-Key` middleware ships nothing publicly yet, so the
migration is "delete the middleware, replace the handlers." Concretely:

- `pinner/src/http.rs`: drop `require_api_key`, add a receipt-and-
  signature verifier middleware that materialises `(receipt,
  bound_to)` on the request extensions.
- `pinner/src/db.rs`: rename `PinnedRow` fields per the schema above.
  `customer: Option<String>` goes away.
- `pinner/src/cli.rs`: drop `api_key`. Add `operator_key:
  PathBuf` (path to the pinner's ed25519 private key on disk; default
  `/var/lib/samizdat/pinner/operator.ed25519`). Pinner generates and
  persists the key on first start if missing.
- `samizdat` CLI gains `samizdat pinner pin <pinner_url> <series>
  --days N --bound-to <key>` for the publisher side. Reads / saves
  the returned receipt in `~/.samizdat/pin-receipts/<receipt_id>.toml`.

`~200 lines` of pinner change; `~150 lines` of CLI side. No node-side
change. No chain dependency.

## Size: the second axis of a pinning slot (and a node-side DoS fix)

A pinning slot has two dimensions, not one. V1's `days` budget alone
leaves the *byte* dimension unbounded, which is a real DoS today --
not just for pinners but for any node that ever calls
`samizdat subscription new`. The pinner amplification makes the
attack interesting but does not introduce it.

### The attack as it stands today (pre-pinner)

A node subscribes to series X via `FullInventory`. Publisher pushes
an edition whose total inventory is, say, 1 TB. `SeriesRef::advance`
(`node/src/models/series.rs:414-426`) places `BookmarkType::Reference`
on every inventory item. The eager fetch begins, the disk fills,
and vacuum *cannot reclaim* because every inventory object is
bookmarked. Other content -- including other pinned series, user-
bookmarked objects -- gets evicted to make room (or vacuum reports
`VacuumStatus::Insufficient` and the node stalls). One adversarial
publisher takes down everyone else's content.

This is `axis 3` (availability) in
`docs/threat-model.md`'s framing and bites today on the testbed; the
only reason we have not seen it is that the testbed's subscriptions
are to series we trust.

### The node-side fix (independent of pinning)

`Subscription` (in `node/src/models/subscription.rs`) grows a
`max_bytes: Option<u64>` field. `None` falls back to the operator's
config default (`max_storage_per_subscription`, defaulting to e.g.
100 MB). The CLI `samizdat subscription new <key>` gains
`--max-bytes N`.

Enforcement happens during the chunk-receive path
(`node/src/system/transport/file_transfer/` or whichever lower
layer materialises bytes from candidates). Per-subscription running
total accumulates across the in-flight edition refresh. When the
total exceeds `max_bytes`:

1. Abort all in-flight fetches for objects in this edition.
2. Do NOT call `SeriesRef::advance` -- the old edition stays current.
3. Log loudly with the publisher key and the observed size.
4. Vacuum eventually reclaims partial bytes from aborted objects
   (they were never bookmarked because advance never ran).

The publisher's node sees a clean machine-readable error on its
edition announcement. The subscriber's old edition is intact; the
oversized edition is rejected. No partial-degradation state.

### The pinner-side surface

With the node hook in place, the pinner just passes a value through.
`PinReceipt` carries `max_bytes: u64`; the pinner's
`add_subscription` call to its local node includes the cap in the
`POST /_subscriptions/` body. Same enforcement, same outcome -- the
pinner just gets to *price* the cap and bind it to a receipt rather
than relying on a static operator default.

Add to the receipt struct:

```rust
struct PinReceipt {
    series_key: Key,
    expires_at: DateTime<Utc>,
    bound_to: Key,
    receipt_id: Hash,
    /// Maximum total bytes the series may occupy on this pinner.
    /// Editions whose total inventory exceeds this cap are refused
    /// at the subscriber's node before the eager fetch can fill
    /// disk. Pricing falls out naturally as
    /// `price_per_gb_day * (max_bytes / GB) * days_remaining`.
    max_bytes: u64,
    pinner_sig: Signature,
}
```

`max_bytes` is signed as part of the receipt; the publisher cannot
upgrade their cap without buying a fresh receipt.

### Upgrade flow

Publisher hits the cap, wants more headroom. They `POST /renew`
with `new_max_bytes`. Pinner runs the funding check at the new
tier, issues a fresh receipt, the old `receipt_id` is superseded
in the LMDB row. The publisher's node retries the edition advance
under the new cap.

### Operator-side safety net

Pinner config gets `max_bytes_per_pin: u64` (e.g. 100 GB).
Pinner refuses to issue receipts above this. Bounds the worst case
a single customer can demand from one operator, independent of
how the funding model evolves.

### Sequencing

The node-side enforcement is the actual fix and ships independently
of the receipt scheme. Pinner V1.5 then becomes a thin wrapper
that selects per-customer caps; without the node hook, the receipt
machinery is unable to enforce its own `max_bytes` field.

### What counts as "size"

Three honest candidates:

- *Sum of unique-object sizes in the current edition* (computed
  from `ObjectMetadata.size`). What we'd ship. Slight overestimate
  when editions share objects, but the overestimate is on the
  operator's side (charges for more than disk holds), which is the
  safer direction for billing errors.
- *Sum of unique chunks across all editions ever held*. Most honest
  for cumulative billing; requires per-subscription bookkeeping
  beyond the current scheme. Defer.
- *Live disk occupancy* via the existing
  `Table::ObjectStatistics` sum. Cheap to compute but reflects
  vacuum state, which is the wrong layer for a publisher-facing
  cap.

### Inventory format unchanged

`Inventory` stays `BTreeMap<ItemPathBuf, Hash>`. Sizes are obtained
by fetching `ObjectMetadata` per inventory item (small: chunk count
+ chunk hashes + header) before launching the full fetch pass.
Authenticated via the existing content-hash chain; an adversarial
publisher cannot lie about an object's size because the chunk
merkle root is part of the object hash.

## Open questions

1. **Where do receipts live?** Three sensible options:
   - Held privately by `bound_to`. Lose the receipt, lose the slot.
   - Published in the series's own next edition under `_pins/`. Public,
     auditable, recoverable.
   - Both: receipts can be published OR held privately; pinner accepts
     either.
   The third is most flexible and zero extra code on the pinner.

2. **Macaroons as a free upgrade path.** Receipts could carry attached
   caveats ("valid only for renewals under 30 days at a time", "only
   delegatable once more"). Pinner verifies the chain. Useful for
   hierarchical delegation; almost certainly overkill for V1.5.

3. **Anonymous credentials at redemption time.** BBS+ / CL signatures
   let the receipt holder prove "I hold a valid receipt for series X"
   without revealing *which* receipt. Defer until there is an actual
   threat model that demands log-time anonymity.

4. **Receipt revocation.** Today's design: the pinner cannot revoke a
   receipt once issued; expiry is the only invalidation. Sufficient
   for the funding models we have in mind. If the operator ever
   needs early revocation (court order, fraud), a small revocation
   list keyed by `receipt_id` is a one-screen change.

5. **Receipt format on the wire.** Bincode of the struct above is the
   obvious choice; base32-encoded for human handling. Should be a
   single short string, not JSON.

## Out of scope

- Polygon-based receipt issuance. The chain-watched funding path is a
  separate epic; receipts are the same shape whether the funding check
  reads from a config allowlist or from chain state.
- Browser-side receipt signing. Wallets sign; browsers do not yet.
- Receipt transfer between humans. Out-of-band; the system does not
  need to facilitate it.
