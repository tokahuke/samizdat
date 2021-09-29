use crate::cli;

use semver::{Comparator, Op, Version, VersionReq};

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

pub fn db<'a>() -> &'a rocksdb::DB {
    unsafe { DB.as_ref().expect("db not initialized") }
}

pub fn init_db() -> Result<(), crate::Error> {
    // Database options:
    let mut db_opts = rocksdb::Options::default();
    db_opts.create_missing_column_families(true);
    db_opts.create_if_missing(true);

    // Open with column families:
    let db = rocksdb::DB::open_cf(
        &db_opts,
        &cli().db_path,
        &vec![
            Table::Global,
            Table::Dependencies,
            Table::Objects,
            Table::ObjectMetadata,
            Table::ObjectChunks,
            Table::ObjectStatistics,
            Table::Collections,
            Table::CollectionMetadata,
            Table::CollectionItems,
            Table::CollectionItemLocators,
            Table::Series,
            Table::SeriesItems,
            Table::SeriesOwners,
            Table::RecentNonces,
        ]
        .into_iter()
        .map(Table::name)
        .collect::<Vec<_>>(),
    )?;

    // Set static:
    unsafe {
        DB = Some(db);
    }

    // Configure version:
    configure_version();

    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub enum Table {
    /// Global, singleton information.
    Global,
    /// Dependencies between entities in the database.
    Dependencies,
    /// The list of all inscribed hashes.
    Objects,
    /// The map of all object (out-of-band) metadata, indexed by object hash.
    ObjectMetadata,
    /// The table of all chunks, indexed by chunk hash.
    ObjectChunks,
    /// Statistics on object usage.
    ObjectStatistics,
    /// The list of all known collections.
    Collections,
    /// The list of all collection metadata.
    CollectionMetadata,
    /// The list of all collection items, indexed by item hash.
    CollectionItems,
    /// The lit of all collection item hashes, indexed by locator.
    CollectionItemLocators,
    /// The list of all series.
    Series,
    /// The list of all most common association between collections and series.
    SeriesItems,
    /// The list of series owners: pieces of information which allows the
    /// publication of a new version of a series in the network.
    SeriesOwners,
    /// A set of current recent nonces.
    RecentNonces,
}

impl Table {
    pub fn name(self) -> String {
        format!("{:?}", self)
    }

    pub fn get<'a>(self) -> &'a rocksdb::ColumnFamily {
        let db = db();
        db.cf_handle(&format!("{:?}", self))
            .expect("column family exists")
    }
}
