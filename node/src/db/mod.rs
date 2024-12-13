//! Application-specific management of the RocksDB database.

mod migrations;

use serde_derive::{Deserialize, Serialize};
use std::fmt::Display;
use std::sync::LazyLock;
use std::{collections::BTreeSet, sync::OnceLock};
use strum::IntoEnumIterator;
use strum_macros::{EnumIter, IntoStaticStr};

use crate::cli;

/// A lock that allows the _write_ holder to perform deletion operations on chunks.
/// This lock must be held in `write` mode by all operations attempting to
/// non-atomically __delete__ chunks from the database. It must also be held in
/// `read` mode by all operations writing to chunks.
pub static CHUNK_RW_LOCK: LazyLock<tokio::sync::RwLock<()>> =
    LazyLock::new(tokio::sync::RwLock::default);

/// A set of locks (to reduce lock contention) that allow the holder to write on
/// [`Table::ObjectStatistics`]. This can be blocking because the lock holding
/// operations are very fast.
pub static STATISTICS_MUTEXES: LazyLock<[std::sync::Mutex<()>; 12]> =
    LazyLock::new(Default::default);

/// The handle to the RocksDB database.
static DB: OnceLock<rocksdb::DB> = OnceLock::new();

/// Retrieves a reference to the RocksDB database. Must be called after initialization.
pub fn db<'a>() -> &'a rocksdb::DB {
    DB.get().expect("database should be initialized first")
}

/// Initializes the RocksDB for use by the Samizdat node.
pub fn init_db() -> Result<(), crate::Error> {
    tracing::info!("Starting RocksDB");

    let db_path = format!("{}/db", cli().data.to_str().expect("path is not a string"));

    // Make sure all column families are initialized:
    // (ignore db error in this case because db may not exist; let it explode later...)
    let existing_cf_names = rocksdb::DB::list_cf(&rocksdb::Options::default(), &db_path)
        .unwrap_or_default()
        .into_iter()
        .collect::<BTreeSet<_>>();
    let needed_cf_names = Table::names().collect::<BTreeSet<_>>();
    let useless_cfs = existing_cf_names
        .difference(&needed_cf_names)
        .map(|cf_name| rocksdb::ColumnFamilyDescriptor::new(cf_name, rocksdb::Options::default()));

    // Database options:
    let mut db_opts = rocksdb::Options::default();
    db_opts.create_missing_column_families(true);
    db_opts.create_if_missing(true);

    // Open with _all_ column families (otherwise RocksDB will complain. Yes, that is the
    // default behavior. No, you can't change that):
    let db = rocksdb::DB::open_cf_descriptors(
        &db_opts,
        &db_path,
        Table::descriptors().chain(useless_cfs),
    )?;

    DB.set(db).ok();

    // Run possible migrations (needs DB set, but still requires exclusive access):
    tracing::info!("RocksDB up. Running migrations...");
    migrations::migrate()?;
    tracing::info!("... done running all migrations.");

    Ok(())
}

/// All column families in the RocksDB database.
#[derive(Debug, Clone, Copy, EnumIter, IntoStaticStr)]
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

impl Display for Table {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", <&'static str>::from(self))
    }
}

/// An alias for the merge function pointer.
type MergeFunction = fn(&[u8], Option<&[u8]>, &rocksdb::MergeOperands) -> Option<Vec<u8>>;

impl Table {
    /// An iterator for all column family descriptors in the database.
    fn descriptors() -> impl Iterator<Item = rocksdb::ColumnFamilyDescriptor> {
        Table::iter().map(Table::descriptor)
    }

    /// An iterator for all column family names in the database.
    fn names() -> impl Iterator<Item = String> {
        Table::iter().map(|table| table.to_string())
    }

    /// Merge operator for the column family, if any.
    fn merge_operator(self) -> Option<MergeFunction> {
        match self {
            Table::Bookmarks => Some(MergeOperation::full_merge),
            Table::ObjectChunkRefCount => Some(MergeOperation::full_merge),
            _ => None,
        }
    }

    /// Descriptor for column family initialization.
    fn descriptor(self) -> rocksdb::ColumnFamilyDescriptor {
        let mut column_opts = rocksdb::Options::default();
        let name = self.to_string();

        if let Some(operator) = self.merge_operator() {
            column_opts.set_merge_operator_associative(&name, operator);
        }

        rocksdb::ColumnFamilyDescriptor::new(name, column_opts)
    }

    /// Gets the underlying column family after database initialization.
    pub fn get<'a>(self) -> &'a rocksdb::ColumnFamily {
        let db = db();
        db.cf_handle(self.into()).expect("column family exists")
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
    fn associative(self, other: Self) -> MergeOperation {
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

    /// The full merge operator for rocksDB
    fn try_full_merge(
        _new_key: &[u8],
        existing_val: Option<&[u8]>,
        operands: &rocksdb::MergeOperands,
    ) -> Result<Option<Vec<u8>>, crate::Error> {
        let mut current: MergeOperation = existing_val
            .map(bincode::deserialize)
            .transpose()?
            .unwrap_or_default();

        for operand in operands {
            let right = bincode::deserialize::<MergeOperation>(operand)?;
            current = current.associative(right);
        }

        Ok(Some(bincode::serialize(&current).expect("can serialize")))
    }

    fn full_merge(
        new_key: &[u8],
        existing_val: Option<&[u8]>,
        operands: &rocksdb::MergeOperands,
    ) -> Option<Vec<u8>> {
        match MergeOperation::try_full_merge(new_key, existing_val, operands) {
            Ok(val) => val,
            Err(err) => {
                tracing::error!(
                    "full merge got bad operation for key {} with operands {:?}: {}",
                    base64_url::encode(new_key),
                    operands
                        .into_iter()
                        .map(base64_url::encode)
                        .collect::<Vec<_>>(),
                    err
                );
                existing_val.map(Vec::from)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge() {
        tracing_subscriber::fmt().init();

        crate::cli::init_cli().unwrap();
        init_db().unwrap();

        db().merge_cf(
            Table::Bookmarks.get(),
            b"a",
            bincode::serialize(&MergeOperation::Set(1)).unwrap(),
        )
        .unwrap();

        let value: MergeOperation =
            bincode::deserialize(&*db().get_cf(Table::Bookmarks.get(), b"a").unwrap().unwrap())
                .unwrap();
        assert_eq!(value.eval_on_zero(), 1);

        db().merge_cf(
            Table::Bookmarks.get(),
            b"a",
            bincode::serialize(&MergeOperation::Increment(1)).unwrap(),
        )
        .unwrap();

        let value: MergeOperation =
            bincode::deserialize(&*db().get_cf(Table::Bookmarks.get(), b"a").unwrap().unwrap())
                .unwrap();
        assert_eq!(value.eval_on_zero(), 2);

        db().merge_cf(
            Table::Bookmarks.get(),
            b"a",
            bincode::serialize(&MergeOperation::Increment(-2)).unwrap(),
        )
        .unwrap();

        tracing::info!(
            "{:?}",
            bincode::deserialize::<MergeOperation>(
                &db().get_cf(Table::Bookmarks.get(), b"a").unwrap().unwrap()
            )
            .unwrap()
        );
    }
}
