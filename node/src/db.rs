//! Application-specific management of the RocksDB database.

use semver::{Comparator, Op, Version, VersionReq};
use serde_derive::{Deserialize, Serialize};

use crate::cli;

/// The handle to the RocksDB database.
static mut DB: Option<rocksdb::DB> = None;

/// Sets the version of the DB or panics if the version is incompatible.
fn configure_version() {
    let current_version = env!("CARGO_PKG_VERSION")
        .parse::<Version>()
        .expect("bad crate version from env");
    let requirement = VersionReq {
        comparators: vec![Comparator {
            op: Op::Caret,
            major: current_version.major,
            minor: Some(current_version.minor),
            patch: Some(current_version.patch),
            pre: current_version.pre,
        }],
    };
    log::info!("Checking db version (expecting {})", requirement);

    if let Some(version) = db()
        .get_cf(Table::Global.get(), b"version")
        .expect("db error")
    {
        let db_version = String::from_utf8_lossy(&version)
            .parse::<Version>()
            .expect("bad db version");
        assert!(
            requirement.matches(&db_version),
            "DB version is {}; required {}",
            db_version,
            requirement
        );
    } else {
        db().put_cf(
            Table::Global.get(),
            b"version",
            env!("CARGO_PKG_VERSION").as_bytes(),
        )
        .expect("db error");
    }
}

/// Retrieives a reference to the RocksDB database. Must be called after initialization.
pub fn db<'a>() -> &'a rocksdb::DB {
    unsafe { DB.as_ref().expect("db not initialized") }
}

/// Initializes the RocksDB for use by the Samizdat hub.
pub fn init_db() -> Result<(), crate::Error> {
    // Database options:
    let mut db_opts = rocksdb::Options::default();
    db_opts.create_missing_column_families(true);
    db_opts.create_if_missing(true);

    // Open with column families:
    let mut db = rocksdb::DB::open_cf_descriptors(
        &db_opts,
        &cli().db_path,
        vec![
            Table::Global,
            Table::Objects,
            Table::ObjectMetadata,
            Table::ObjectChunks,
            Table::ObjectStatistics,
            Table::Bookmarks,
            Table::Collections,
            Table::CollectionMetadata,
            Table::CollectionItems,
            Table::CollectionItemLocators,
            Table::Series,
            Table::Editions,
            Table::SeresFreshnesses,
            Table::SeriesOwners,
            Table::Subscriptions,
            Table::RecentNonces,
        ]
        .into_iter()
        .map(Table::descriptor),
    )?;

    // Run possible migrations:
    migrate(&mut db)?;

    // Set static:
    unsafe {
        DB = Some(db);
    }

    // Configure version:
    configure_version();

    Ok(())
}

/// All column families in the RocksDB database.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum Table {
    /// Global, singleton information.
    Global,
    /// The list of all inscribed hashes.
    Objects,
    /// The map of all object (out-of-band) metadata, indexed by object hash.
    ObjectMetadata,
    /// The table of all chunks, indexed by chunk hash.
    ObjectChunks,
    /// Statistics on object usage.
    ObjectStatistics,
    /// List of dependencies on objects, which prevent automatic deletion.
    Bookmarks,
    /// The list of all known collections.
    Collections, // DEPRECATED
    /// The list of all collection metadata.
    CollectionMetadata, // DEPRECATED
    /// The list of all collection items, indexed by item hash.
    CollectionItems,
    /// The lit of all collection item hashes, indexed by locator.
    CollectionItemLocators,
    /// The list of all series.
    Series,
    /// The list of all most common association between collections and series.
    Editions,
    /// The last refresh dates from each series.
    SeresFreshnesses,
    /// The list of series owners: pieces of information which allows the
    /// publication of a new version of a series in the network.
    SeriesOwners,
    /// Subscription to series on the network.
    Subscriptions,
    /// A set of current recent nonces.
    RecentNonces,
}

/// An aliase fot the merge function pointer.
type MergeFunction = fn(&[u8], Option<&[u8]>, &mut rocksdb::MergeOperands) -> Option<Vec<u8>>;

impl Table {
    /// Name of the corresponding column family.
    fn name(self) -> String {
        format!("{:?}", self)
    }

    /// Merge operator for the column family, if any.
    fn merge_operator(self) -> Option<MergeFunction> {
        match self {
            Table::Bookmarks => Some(MergeOperation::full_merge),
            _ => None,
        }
    }

    /// Descriptor for column family initialization.
    fn descriptor(self) -> rocksdb::ColumnFamilyDescriptor {
        let mut column_opts = rocksdb::Options::default();
        let name = self.name();

        if let Some(operator) = self.merge_operator() {
            column_opts.set_merge_operator_associative(&name, operator);
        }

        rocksdb::ColumnFamilyDescriptor::new(self.name(), column_opts)
    }

    /// Gets the underlying column family after database initialization.
    pub fn get<'a>(self) -> &'a rocksdb::ColumnFamily {
        let db = db();
        db.cf_handle(&format!("{:?}", self))
            .expect("column family exists")
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
    /// Evalues the resulting operation from successive operations.
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
        operands: &mut rocksdb::MergeOperands,
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
        operands: &mut rocksdb::MergeOperands,
    ) -> Option<Vec<u8>> {
        match MergeOperation::try_full_merge(new_key, existing_val, operands) {
            Ok(val) => val,
            Err(err) => {
                log::error!(
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

/// Runs database migration tasks.
///
/// # Note
///
/// By now, just reserved for future use.
fn migrate(db: &mut rocksdb::DB) -> Result<(), crate::Error> {
    //db.drop_cf(&Table::Dependencies.name())?;
    let _ = db; // suppresses unused variable warning.

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge() {
        let _ = crate::logger::init_logger();

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

        log::info!(
            "{:?}",
            bincode::deserialize::<MergeOperation>(
                &db().get_cf(Table::Bookmarks.get(), b"a").unwrap().unwrap()
            )
            .unwrap()
        );
    }
}
