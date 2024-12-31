//! The Samizdat Hub database, based on top of RocksDb.

mod migrations;

use samizdat_common::db::Migration;
use strum_macros::{IntoStaticStr, VariantArray};

pub use samizdat_common::db::init_db;

/// All column families in the RocksDB database.
#[derive(Debug, Clone, Copy, VariantArray, IntoStaticStr)]
pub enum Table {
    /// Global, singleton information.
    Global,
    /// The list of applied migrations.
    Migrations,
    /// The list of all recent nonces. This is to mitigate replay attacks.
    RecentNonces,
    /// Blacklisted IP addresses
    BlacklistedIps,
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
