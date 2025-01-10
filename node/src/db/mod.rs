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

/// Possible merge operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeOperation {
    /// Increments an `i16` key by some number.
    Increment(i16),
    /// Sets an `i16`.
    Set(i16),
}

impl Default for MergeOperation {
    fn default() -> MergeOperation {
        MergeOperation::Increment(0)
    }
}

impl MergeOperation {
    /// Evaluates the resulting operation from successive operations.
    pub fn associative(self, other: Self) -> MergeOperation {
        match (self, other) {
            (MergeOperation::Increment(inc1), MergeOperation::Increment(inc2)) => {
                MergeOperation::Increment(inc1 + inc2)
            }
            (MergeOperation::Set(val), MergeOperation::Increment(inc)) => {
                MergeOperation::Set(val + inc)
            }
            (_, MergeOperation::Set(val)) => MergeOperation::Set(val),
        }
    }

    /// Does the merge operation dance for one operand.
    pub fn eval_on_zero(self) -> i16 {
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
