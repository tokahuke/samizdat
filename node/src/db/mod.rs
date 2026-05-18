//! Application-specific management of the RocksDB database.

mod migrations;

pub use samizdat_common::db::init_db;

use samizdat_common::db::Migration;
use serde_derive::{Deserialize, Serialize};
use strum::VariantArray;
use strum_macros::IntoStaticStr;

/// All column families in the RocksDB database.
#[derive(Debug, Clone, Copy, IntoStaticStr, VariantArray)]
#[non_exhaustive]
pub enum Table {
    /// Global, singleton information.
    Global,
    /// The list of applied migrations.
    Migrations,
    /// The list of all inscribed hashes.
    Objects,
    /// The map of all object (out-of-band) metadata, indexed by object hash.
    ObjectMetadata,
    /// The table of all chunks, indexed by chunk hash.
    ObjectChunks,
    /// The table of all chunks, indexed by chunk hash.
    ObjectChunkRefCount,
    /// Statistics on object usage.
    ObjectStatistics,
    /// List of dependencies on objects, which prevent automatic deletion.
    Bookmarks,
    /// The list of all collection items, indexed by item hash.
    CollectionItems,
    /// The lit of all collection item hashes, indexed by locator.
    CollectionItemLocators,
    /// The list of all series.
    Series,
    /// The list of all most common association between collections and series.
    Editions,
    /// The last refresh dates from each series.
    SeriesFreshnesses,
    /// The list of series owners: pieces of information which allows the
    /// publication of a new version of a series in the network.
    SeriesOwners,
    /// (DEPRECATED)
    Identities,
    /// Subscription to series on the network.
    Subscriptions,
    /// A set of current recent nonces.
    RecentNonces,
    /// Access rights granted for each entity to the local Samizdat node.
    AccessRights,
    /// General key-value store for application (because `LocalStorage` is broken in Samizdat).
    KVStore,
    /// Specification on which hubs to connect to.
    Hubs,
}

impl samizdat_common::db::Table for Table {
    const MIGRATIONS: Self = Table::Migrations;

    fn base_migration() -> Box<dyn Migration<Self>> {
        Box::new(migrations::BaseMigration)
    }

    fn discriminant(self) -> usize {
        self as usize
    }
}

/// Possible merge operations.
///
/// Widened from `i16` to `i32` so refcounts/bookmark counts don't wrap on objects with
/// many duplicate chunks or series with many editions. At `i16` the cap was 32 767; a
/// single object with that many repeated chunk hashes (~8 GB of homogeneous data) or
/// 32 768 active editions pinning the same object would silently wrap to negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeOperation {
    /// Increments an `i32` key by some number.
    Increment(i32),
    /// Sets an `i32`.
    Set(i32),
}

impl Default for MergeOperation {
    fn default() -> MergeOperation {
        MergeOperation::Increment(0)
    }
}

impl MergeOperation {
    /// Evaluates the resulting operation from successive operations. Uses
    /// `saturating_add` rather than wrapping: at i32 the cap (~2.1B) is so far above
    /// any realistic refcount that saturating is effectively never-reached, but it
    /// guarantees no silent rollover to negative even if it is.
    pub fn associative(self, other: Self) -> MergeOperation {
        match (self, other) {
            (MergeOperation::Increment(inc1), MergeOperation::Increment(inc2)) => {
                MergeOperation::Increment(inc1.saturating_add(inc2))
            }
            (MergeOperation::Set(val), MergeOperation::Increment(inc)) => {
                MergeOperation::Set(val.saturating_add(inc))
            }
            (_, MergeOperation::Set(val)) => MergeOperation::Set(val),
        }
    }

    /// Does the merge operation dance for one operand.
    pub fn eval_on_zero(self) -> i32 {
        match self {
            MergeOperation::Increment(inc) => inc,
            MergeOperation::Set(set) => set,
        }
    }

    /// Creates a function that merges the operation with an existing value.
    pub fn merger(self) -> impl Fn(Option<&[u8]>) -> Vec<u8> {
        move |maybe_value: Option<&[u8]>| {
            let Some(serialized_value) = maybe_value else {
                return bincode::serialize(&self).expect("can serialize");
            };
            let old: MergeOperation =
                bincode::deserialize(serialized_value).expect("value was correctly encoded");
            let new = old.associative(self);

            bincode::serialize(&new).expect("can serialize")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MergeOperation::{self, Increment, Set};

    /// Regression test for B9; refcount/bookmark merge math used to wrap on i16.
    /// At i32 the cap is so far above realistic usage that saturating is effectively
    /// unreachable, but we still verify it saturates (rather than wraps to negative)
    /// at the boundary.
    #[test]
    fn associative_saturates_at_i32_max() {
        let big = Increment(i32::MAX);
        match big.associative(Increment(1)) {
            Increment(v) => assert_eq!(v, i32::MAX),
            other => panic!("expected Increment, got {other:?}"),
        }
        match Set(i32::MAX).associative(Increment(1)) {
            Set(v) => assert_eq!(v, i32::MAX),
            other => panic!("expected Set, got {other:?}"),
        }
    }

    #[test]
    fn associative_saturates_at_i32_min() {
        match Increment(i32::MIN).associative(Increment(-1)) {
            Increment(v) => assert_eq!(v, i32::MIN),
            other => panic!("expected Increment, got {other:?}"),
        }
    }

    /// Sanity: normal addition is unchanged.
    #[test]
    fn associative_normal_addition() {
        assert!(matches!(
            Increment(5).associative(Increment(3)),
            Increment(8)
        ));
        assert!(matches!(Set(10).associative(Increment(3)), Set(13)));
        assert!(matches!(Set(10).associative(Set(99)), Set(99)));
    }

    /// Eval-on-zero returns the carried scalar regardless of variant.
    #[test]
    fn eval_on_zero() {
        assert_eq!(Increment(7i32).eval_on_zero(), 7);
        assert_eq!(Set(-3i32).eval_on_zero(), -3);
    }

    /// B5 cross-check: an i16-style refcount close to the old cap is now fine.
    #[test]
    fn refcount_past_old_i16_cap_does_not_wrap() {
        // 40 000 successive +1s would have wrapped at i16; on i32 it's a no-op.
        let mut op = MergeOperation::default();
        for _ in 0..40_000 {
            op = op.associative(Increment(1));
        }
        assert_eq!(op.eval_on_zero(), 40_000);
    }
}
