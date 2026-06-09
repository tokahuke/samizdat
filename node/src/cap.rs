//! Size-cap abstraction for object size enforcement.
//!
//! Three concerns historically had three separate implementations: the
//! per-object `max_content_size` check at
//! `transport/file_transfer/messages.rs::validate`, the per-subscription
//! `max_bytes` cap on `Subscription`, and the global `max_storage`
//! budget that vacuum reads off `Table::ObjectStatistics`. They are
//! siblings of the same idea -- "how many more bytes may land on disk
//! for this scope?" -- so this module unifies them behind a `Cap` trait
//! with four flavours that compose via `Composite`.
//!
//! ## Flavours
//!
//! - [`Unbounded`]: always-Ok; used where the caller does not need a scope-specific cap.
//! - [`Budget`]: a depleting budget with atomic decrement on reserve and atomic increment
//!   on release. The live counter is in-memory because reserves must be lock-free; LMDB's
//!   single-writer model would serialise every reserve through the writer lock. Ground
//!   truth lives on disk (`Table::ObjectStatistics`, `Table::Subscriptions`) and is what
//!   `reconstruct_from_disk` reads.
//! - [`SizeLimit`]: a stateless `size > limit` gate. Used for the per-object size cap (a
//!   single reserve may not exceed the limit; no budget depletion).
//! - [`Composite`]: all-or-nothing across N children. First failure rolls back the prior
//!   successes via the existing `Reservation::Drop` impl; on full success a single
//!   bundled `Reservation` owns the children, and its `commit()` / `Drop` propagates.
//!
//! ## RAII
//!
//! [`Reservation`] is the handle returned by a successful `reserve`.
//! Its `Drop` impl releases the size back to the cap by default. The
//! `commit()` method makes the decrement permanent (used after the
//! object has been written to disk, at which point the budget should
//! stay decremented until vacuum drops the object and calls `release`
//! explicitly).

use std::{
    fmt,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use samizdat_common::{
    Key,
    db::{Table as _, readonly_tx},
};
use thiserror::Error;

use crate::cli::cli;

/// A `reserve` was rejected. `label` identifies which cap rejected;
/// callers log it for telemetry. The distinction between a depleting
/// budget and a stateless size limit isn't load-bearing at the call
/// site (both are "asked more than the current bound allowed"), so
/// one variant carries both.
#[derive(Debug, Clone, Error)]
#[error(
    "cap {label} exceeded: asked {}, bound {}",
    human_bytes(*.asked), human_bytes(*.bound)
)]
pub struct CapError {
    pub label: String,
    pub asked: usize,
    pub bound: usize,
}

/// Format a byte count with the largest fitting SI unit (GB, MB, KB)
/// or plain bytes below 1 KB. Used in `CapError`'s Display impl so
/// operator-facing messages read in human units, not raw byte
/// counts.
pub fn human_bytes(n: usize) -> String {
    const UNITS: &[(usize, &str)] = &[(1_000_000_000, "GB"), (1_000_000, "MB"), (1_000, "KB")];
    for &(threshold, unit) in UNITS {
        if n >= threshold {
            return format!("{:.2} {unit}", n as f64 / threshold as f64);
        }
    }
    format!("{n} B")
}

impl From<CapError> for crate::Error {
    fn from(err: CapError) -> crate::Error {
        crate::Error::Message(err.to_string())
    }
}

/// A scope-specific or ambient size cap that supports atomic
/// reservation. Implementations must be cheap to clone (the trait is
/// always used through `Arc<dyn Cap>`).
pub trait Cap: Send + Sync + fmt::Debug {
    /// Atomically reserve `size` bytes. Either succeeds (decrementing
    /// the budget or passing the size gate) or returns `Err` with the
    /// cap unchanged.
    fn reserve(self: Arc<Self>, size: usize) -> Result<Reservation, CapError>;
    /// Permanently release `size` bytes. Called by vacuum when an
    /// object is dropped; the in-flight `Reservation` path uses
    /// `Drop` to call this on the failure path.
    fn release(&self, size: usize);
}

/// Owning handle to an in-flight or committed reservation. Drops back
/// to the cap by default; `commit()` makes the decrement permanent.
#[derive(Debug)]
pub struct Reservation {
    inner: ReservationInner,
    released: AtomicBool,
}

#[derive(Debug)]
enum ReservationInner {
    Single { cap: Arc<dyn Cap>, size: usize },
    Bundle { children: Vec<Reservation> },
}

impl Reservation {
    fn single(cap: Arc<dyn Cap>, size: usize) -> Reservation {
        Reservation {
            inner: ReservationInner::Single { cap, size },
            released: AtomicBool::new(false),
        }
    }

    fn bundle(children: Vec<Reservation>) -> Reservation {
        Reservation {
            inner: ReservationInner::Bundle { children },
            released: AtomicBool::new(false),
        }
    }

    /// Make the reservation permanent. The reserved bytes stay
    /// decremented from the underlying cap until an explicit
    /// `Cap::release` call (typically from vacuum on object drop).
    /// Idempotent.
    pub fn commit(&self) {
        if self.released.swap(true, Ordering::AcqRel) {
            return;
        }
        if let ReservationInner::Bundle { children } = &self.inner {
            for child in children {
                child.commit();
            }
        }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if self.released.swap(true, Ordering::AcqRel) {
            return;
        }
        if let ReservationInner::Single { cap, size } = &self.inner {
            cap.release(*size);
        }
        // Bundle: each child's own Drop releases as the Vec drops.
    }
}

/// Always-Ok cap. Used as the default `caller_cap` inside `Composite`
/// when the import caller has no scope-specific cap to add.
#[derive(Debug)]
pub struct Unbounded;

impl Cap for Unbounded {
    fn reserve(self: Arc<Self>, _size: usize) -> Result<Reservation, CapError> {
        Ok(Reservation::single(self, 0))
    }
    fn release(&self, _size: usize) {}
}

/// Depleting budget. The remaining-bytes counter is atomic and lives
/// in memory; ground truth lives on disk and is what
/// `reconstruct_from_disk` walks at boot to compute the initial value.
#[derive(Debug)]
pub struct Budget {
    remaining: AtomicUsize,
    label: String,
}

impl Budget {
    pub fn new(label: impl Into<String>, initial_remaining: usize) -> Arc<Self> {
        Arc::new(Budget {
            remaining: AtomicUsize::new(initial_remaining),
            label: label.into(),
        })
    }

    #[cfg(test)]
    pub fn remaining(&self) -> usize {
        self.remaining.load(Ordering::Acquire)
    }

    /// Replace the running `remaining` counter wholesale. Used by
    /// `reconstruct_from_disk` to seed this budget against the
    /// on-disk truth at startup; not for the steady-state path
    /// (which goes through `Cap::reserve` / `Cap::release`).
    pub fn set(&self, remaining: usize) {
        self.remaining.store(remaining, Ordering::Release);
    }
}

impl Cap for Budget {
    fn reserve(self: Arc<Self>, size: usize) -> Result<Reservation, CapError> {
        // Compare-and-swap loop: load current, check headroom, try to
        // decrement; retry on contention. Lock-free.
        loop {
            let current = self.remaining.load(Ordering::Acquire);
            if size > current {
                return Err(CapError {
                    label: self.label.clone(),
                    asked: size,
                    bound: current,
                });
            }
            let new_remaining = current - size;
            if self
                .remaining
                .compare_exchange_weak(current, new_remaining, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Ok(Reservation::single(self.clone(), size));
            }
        }
    }

    fn release(&self, size: usize) {
        self.remaining.fetch_add(size, Ordering::AcqRel);
    }
}

/// Stateless `size > limit` gate. Composes inside `Composite` next to
/// budget caps without depleting.
#[derive(Debug)]
pub struct SizeLimit {
    limit: usize,
    label: String,
}

impl SizeLimit {
    pub fn new(label: impl Into<String>, limit: usize) -> Arc<Self> {
        Arc::new(SizeLimit {
            limit,
            label: label.into(),
        })
    }
}

impl Cap for SizeLimit {
    fn reserve(self: Arc<Self>, size: usize) -> Result<Reservation, CapError> {
        if size > self.limit {
            Err(CapError {
                label: self.label.clone(),
                asked: size,
                bound: self.limit,
            })
        } else {
            Ok(Reservation::single(self, 0))
        }
    }
    fn release(&self, _size: usize) {}
}

/// All-or-nothing across N child caps. First failure rolls back the
/// prior successful reservations by dropping them; the trait's `Drop`
/// implementation handles the release.
#[derive(Debug)]
pub struct Composite {
    children: Vec<Arc<dyn Cap>>,
}

impl Composite {
    pub fn new(children: Vec<Arc<dyn Cap>>) -> Arc<Self> {
        Arc::new(Composite { children })
    }
}

impl Cap for Composite {
    fn reserve(self: Arc<Self>, size: usize) -> Result<Reservation, CapError> {
        let mut acquired: Vec<Reservation> = Vec::with_capacity(self.children.len());
        for child in &self.children {
            match child.clone().reserve(size) {
                Ok(reservation) => acquired.push(reservation),
                Err(err) => {
                    // Drop the prior reservations -- their Drop impls
                    // release back to each child cap.
                    drop(acquired);
                    return Err(err);
                }
            }
        }
        Ok(Reservation::bundle(acquired))
    }
    fn release(&self, size: usize) {
        for child in &self.children {
            child.release(size);
        }
    }
}

/// Per-object size limit, equivalent to the old
/// `messages.rs::validate` check against `max_content_size`.
/// Initialised from `cli().max_content_size` (megabytes) on first use.
pub static OBJECT_SIZE_LIMIT: LazyLock<Arc<SizeLimit>> = LazyLock::new(|| {
    let limit_bytes = cli().max_content_size.saturating_mul(1_000_000);
    SizeLimit::new("object_size_limit", limit_bytes)
});

/// Node-wide storage budget. Reconstructed from
/// `Table::ObjectStatistics` at boot via `reconstruct_from_disk`.
pub static NODE_STORAGE_CAP: LazyLock<Arc<Budget>> = LazyLock::new(|| {
    let budget = cli().max_storage.saturating_mul(1_000_000);
    Budget::new("node_storage", budget)
});

/// Reconstruct `NODE_STORAGE_CAP`'s remaining budget from the
/// on-disk truth. Call once at startup, before any reserve happens.
///
/// Sums `ObjectStatistics.size` over **every** persisted object
/// (bookmarked and unowned alike, per `docs/cap-model.md`) and sets
/// the cap's remaining to `max_storage_bytes - that_sum`,
/// saturating at zero so a node already over its configured cap
/// (e.g. operator lowered `max_storage` between runs) starts with
/// zero budget and rejects new commitments until vacuum drains
/// enough.
pub fn reconstruct_from_disk() -> Result<(), crate::Error> {
    let mut on_disk: usize = 0;
    readonly_tx(|tx| {
        crate::db::Table::ObjectStatistics
            .range::<_, [u8; 0]>(..)
            .for_each(tx, |_, value| {
                let stats: crate::models::ObjectStatistics = bincode::deserialize(value)?;
                on_disk = on_disk.saturating_add(stats.size());
                Ok::<Option<()>, crate::Error>(None)
            })
    })?;

    let max_storage_bytes = cli().max_storage.saturating_mul(1_000_000);
    let remaining = max_storage_bytes.saturating_sub(on_disk);
    NODE_STORAGE_CAP.set(remaining);

    tracing::info!(
        "cap::reconstruct_from_disk: max_storage = {}, on_disk = {}, \
         NODE_STORAGE_CAP.remaining seeded to {}",
        human_bytes(max_storage_bytes),
        human_bytes(on_disk),
        human_bytes(remaining),
    );

    Ok(())
}

/// Build a fresh per-refresh `Budget` for the named subscription.
///
/// Stateless by design: every call returns a new `Budget` initialised
/// to the subscription's `max_bytes` (or the operator default). There
/// is no cross-refresh registry. A subscription's cap bounds *one*
/// edition's inventory size; the brief disk overlap between
/// outgoing and incoming editions is absorbed by `NODE_STORAGE_CAP`,
/// not the per-edition budget. Bookmarked objects are the persistent
/// commitment that costs against `NODE_STORAGE_CAP`; non-bookmarked
/// objects are vacuum-reclaimable and do not enter the accounting.
pub fn refresh_cap_for(key: &Key) -> Arc<Budget> {
    let max_bytes_opt = readonly_tx(|tx| {
        crate::models::SubscriptionRef::new(key.clone())
            .get(tx)
            .map(|opt| opt.and_then(|sub| sub.max_bytes()))
    })
    .ok()
    .flatten();
    let bytes = match max_bytes_opt {
        // `Subscription.max_bytes` is already bytes (HTTP intake
        // converted from megabytes); the operator default below is
        // still in megabytes so multiply.
        Some(bytes) => bytes as usize,
        None => cli().default_max_edition_size_mb.saturating_mul(1_000_000),
    };
    Budget::new(format!("subscription:{key}"), bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unbounded_always_succeeds() {
        let cap: Arc<dyn Cap> = Arc::new(Unbounded);
        let r = cap.reserve(usize::MAX).expect("unbounded accepts anything");
        r.commit();
    }

    #[test]
    fn atomic_budget_succeeds_within_budget() {
        let cap = Budget::new("test", 1_000);
        let r = cap.clone().reserve(400).expect("400 fits in 1000");
        assert_eq!(cap.remaining(), 600);
        r.commit();
        assert_eq!(cap.remaining(), 600, "commit does not refund");
    }

    #[test]
    fn atomic_budget_fails_when_over() {
        let cap = Budget::new("test", 1_000);
        let err = cap.clone().reserve(1_001).expect_err("over the budget");
        assert_eq!(err.asked, 1_001);
        assert_eq!(err.bound, 1_000);
        assert_eq!(cap.remaining(), 1_000, "failed reserve does not deplete");
    }

    #[test]
    fn atomic_budget_drop_releases() {
        let cap = Budget::new("test", 1_000);
        let r = cap.clone().reserve(400).unwrap();
        assert_eq!(cap.remaining(), 600);
        drop(r);
        assert_eq!(cap.remaining(), 1_000, "drop restores the reservation");
    }

    #[test]
    fn size_limit_succeeds_under() {
        let cap: Arc<dyn Cap> = SizeLimit::new("test", 1_000);
        cap.reserve(999).expect("under the limit");
    }

    #[test]
    fn size_limit_fails_over() {
        let cap: Arc<dyn Cap> = SizeLimit::new("test", 1_000);
        let err = cap.reserve(1_001).expect_err("over the limit");
        assert_eq!(err.asked, 1_001);
        assert_eq!(err.bound, 1_000);
    }

    #[test]
    fn composite_all_succeeds_when_all_pass() {
        let budget = Budget::new("budget", 1_000);
        let limit = SizeLimit::new("limit", 500);
        let composite = Composite::new(vec![budget.clone() as Arc<dyn Cap>, limit]);
        let r = composite.reserve(400).expect("400 fits both");
        assert_eq!(budget.remaining(), 600);
        r.commit();
        assert_eq!(budget.remaining(), 600);
    }

    #[test]
    fn composite_rolls_back_on_failure() {
        let budget = Budget::new("budget", 1_000);
        let limit = SizeLimit::new("limit", 100); // strict
        let composite = Composite::new(vec![budget.clone() as Arc<dyn Cap>, limit]);
        let _err = composite.reserve(500).expect_err("limit rejects 500");
        assert_eq!(
            budget.remaining(),
            1_000,
            "budget that succeeded is rolled back when limit fails"
        );
    }

    #[test]
    fn composite_drop_releases_all_children() {
        let a = Budget::new("a", 1_000);
        let b = Budget::new("b", 1_000);
        let composite = Composite::new(vec![a.clone() as Arc<dyn Cap>, b.clone() as Arc<dyn Cap>]);
        let r = composite.reserve(300).unwrap();
        assert_eq!(a.remaining(), 700);
        assert_eq!(b.remaining(), 700);
        drop(r);
        assert_eq!(a.remaining(), 1_000);
        assert_eq!(b.remaining(), 1_000);
    }

    #[test]
    fn composite_commit_keeps_children_decremented() {
        let a = Budget::new("a", 1_000);
        let b = Budget::new("b", 1_000);
        let composite = Composite::new(vec![a.clone() as Arc<dyn Cap>, b.clone() as Arc<dyn Cap>]);
        let r = composite.reserve(300).unwrap();
        r.commit();
        assert_eq!(a.remaining(), 700);
        assert_eq!(b.remaining(), 700);
    }
}
