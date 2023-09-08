//! The Samizdat Hub database, based on top of RocksDb.

mod migrations;

use std::collections::BTreeSet;
use std::fmt::Display;
use strum::IntoEnumIterator;
use strum_macros::{EnumIter, IntoStaticStr};

use crate::CLI;

/// The handle to the RocksDB database.
static mut DB: Option<rocksdb::DB> = None;

/// Retrieves a reference to the RocksDB database. Must be called after initialization.
pub fn db<'a>() -> &'a rocksdb::DB {
    unsafe { DB.as_ref().expect("db not initialized") }
}

/// Initializes the RocksDB for use by the Samizdat hub.
pub fn init_db() -> Result<(), crate::Error> {
    log::info!("Starting RocksDB");

    let db_path = format!("{}/db", CLI.data.as_str());

    // Make sure all column families are initialized;
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

    // Set static:
    // SAFETY: this is the only write to this variable an this happens before any reads are done.
    // This is a single-threaded initialization function.
    unsafe {
        DB = Some(db);
    }

    // Run possible migrations (needs DB set, but still requires exclusive access):
    log::info!("RocksDB up. Running migrations...");
    migrations::migrate()?;
    log::info!("... done running all migrations.");

    Ok(())
}

/// All column families in the RocksDB database.
#[derive(Debug, Clone, Copy, EnumIter, IntoStaticStr)]
pub enum Table {
    /// Global, singleton information.
    Global,
    /// The list of applied migrations.
    Migrations,
    /// The list of all recent nonces. This is to mitigate replay attacks.
    RecentNonces,
}

impl Display for Table {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", <&'static str>::from(self))
    }
}

impl Table {
    /// An iterator for all column family descriptors in the database.
    fn descriptors() -> impl Iterator<Item = rocksdb::ColumnFamilyDescriptor> {
        Table::iter().map(Table::descriptor)
    }

    /// An iterator for all column family names in the database.
    fn names() -> impl Iterator<Item = String> {
        Table::iter().map(|table| table.to_string())
    }

    /// Descriptor for column family initialization.
    fn descriptor(self) -> rocksdb::ColumnFamilyDescriptor {
        let column_opts = rocksdb::Options::default();
        let name = self.to_string();

        rocksdb::ColumnFamilyDescriptor::new(name, column_opts)
    }

    /// Gets the underlying column family after database initialization.
    pub fn get<'a>(self) -> &'a rocksdb::ColumnFamily {
        let db = db();
        db.cf_handle(self.into()).expect("column family exists")
    }
}
