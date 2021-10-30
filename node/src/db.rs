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
    fn merge_operator(self) -> Option<(MergeFunction, MergeFunction)> {
        match self {
            Table::Bookmarks => Some((MergeOperation::partial_merge, MergeOperation::full_merge)),
            _ => None,
        }
    }

    /// Descriptor for column family initialization.
    fn descriptor(self) -> rocksdb::ColumnFamilyDescriptor {
        let mut column_opts = rocksdb::Options::default();
        let name = self.name();

        if let Some((partial, full)) = self.merge_operator() {
            column_opts.set_merge_operator(&name, partial, full)
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
#[derive(Debug, Serialize, Deserialize)]
pub enum MergeOperation {
    /// Increments an `i16` key by some number.
    Increment(i16),
    /// Sets an `i16`.
    Set(i16),
}

impl MergeOperation {
    /// Evalues the resulting operation from successive operations.
    fn associative(&self, other: &Self) -> MergeOperation {
        match (self, other) {
            (MergeOperation::Increment(inc1), MergeOperation::Increment(inc2)) => {
                MergeOperation::Increment(inc1 + inc2)
            }
            (MergeOperation::Set(val), MergeOperation::Increment(inc)) => {
                MergeOperation::Set(val + inc)
            }
            (_, MergeOperation::Set(val)) => MergeOperation::Set(*val),
        }
    }

    /// Does the merge operation dance for one operand.
    fn eval(&self, current: Option<Vec<u8>>) -> Result<Option<Vec<u8>>, crate::Error> {
        let r#final = match (self, current) {
            (MergeOperation::Increment(inc), None) => *inc,
            (MergeOperation::Increment(inc), Some(current)) => {
                let count = i16::from_be_bytes([current[0], current[1]]);
                count + inc
            }
            (MergeOperation::Set(val), _) => *val,
        };

        if r#final != 0 {
            Ok(Some(r#final.to_be_bytes().to_vec()))
        } else {
            Ok(None)
        }
    }

    /// The partial merge operator for rockesDB.
    fn partial_merge(
        new_key: &[u8],
        _existing_val: Option<&[u8]>,
        operands: &mut rocksdb::MergeOperands,
    ) -> Option<Vec<u8>> {
        // This is an identity op:
        let mut current = MergeOperation::Increment(0);

        for operand in operands {
            match bincode::deserialize::<MergeOperation>(operand) {
                Err(err) => {
                    log::error!(
                        "partial merge got bad operation for key {} with operand {} at {:?}: {}",
                        base64_url::encode(new_key),
                        base64_url::encode(operand),
                        current,
                        err
                    );
                    break;
                }
                Ok(operation) => current = current.associative(&operation),
            }
        }

        Some(bincode::serialize(&current).expect("can serialize"))
    }

    /// The full merge operator for rocksDB
    fn full_merge(
        new_key: &[u8],
        existing_val: Option<&[u8]>,
        operands: &mut rocksdb::MergeOperands,
    ) -> Option<Vec<u8>> {
        let mut current = existing_val.map(Vec::from);

        for operand in operands {
            match bincode::deserialize::<MergeOperation>(operand) {
                Err(err) => {
                    log::error!(
                        "full merge got bad operation for key {} with operand {}: {}",
                        base64_url::encode(new_key),
                        base64_url::encode(operand),
                        err
                    );
                    return existing_val.map(Vec::from);
                }
                Ok(operation) => match operation.eval(current) {
                    Err(err) => {
                        log::error!("merge operation {:?} failed with {}", operation, err);
                        return existing_val.map(Vec::from);
                    }
                    Ok(next) => {
                        current = next;
                    }
                },
            }
        }

        current
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
