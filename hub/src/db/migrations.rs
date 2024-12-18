//! Migrations for the RocksDb database.

use std::fmt::Debug;

use crate::db;

use super::Table;

/// Runs the all the necessary migrations of the RocksDb database.
pub(super) fn migrate() -> Result<(), crate::Error> {
    BaseMigration.migrate()
}

/// A migration to be run in the database at process start.
trait Migration: Debug {
    fn next(&self) -> Option<Box<dyn Migration>>;
    fn up(&self) -> Result<(), crate::Error>;

    fn is_up(&self) -> Result<bool, crate::Error> {
        let migration_key = format!("{self:?}");
        let value = db().get_cf(Table::Migrations.get(), migration_key.as_bytes())?;
        Ok(value.is_some())
    }

    fn migrate(&self) -> Result<(), crate::Error> {
        if !self.is_up()? {
            let migration_key = format!("{self:?}");

            // This should be atomic, but... oh! dear...
            tracing::info!("Applying migration {self:?}...");
            self.up()?;
            db().put_cf(Table::Migrations.get(), migration_key.as_bytes(), [])?;
            tracing::info!("... done.");
        } else {
            tracing::info!("Migration {self:?} already up.");
        }

        // Tail-recurse:
        if let Some(last) = self.next() {
            last.migrate()?;
        }

        Ok(())
    }
}

/// The original migration of the RocksDb database.
#[derive(Debug)]
struct BaseMigration;

impl Migration for BaseMigration {
    fn next(&self) -> Option<Box<dyn Migration>> {
        None
    }

    fn up(&self) -> Result<(), crate::Error> {
        Ok(())
    }
}
