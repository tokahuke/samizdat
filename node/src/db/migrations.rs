//! Migrations to be run to evolve the schema of the database and ensure forward
//! version compatibility.

use std::fmt::Debug;

use crate::db;

use super::Table;

/// The `&mut DB` guarantees exclusive access to the db, since this type is not clonable.
pub(super) fn migrate() -> Result<(), crate::Error> {
    BaseMigration.migrate()
}

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
            log::info!("Applying migration {self:?}...");
            self.up()?;
            db().put_cf(Table::Migrations.get(), migration_key.as_bytes(), [])?;
            log::info!("... done.");
        } else {
            log::info!("Migration {self:?} already up.");
        }

        // Tail-recurse:
        if let Some(last) = self.next() {
            last.migrate()?;
        }

        Ok(())
    }
}

#[derive(Debug)]
struct BaseMigration;

impl Migration for BaseMigration {
    fn next(&self) -> Option<Box<dyn Migration>> {
        Some(Box::new(CreateChunkRefCount))
    }

    fn up(&self) -> Result<(), crate::Error> {
        Ok(())
    }
}

#[derive(Debug)]
struct CreateChunkRefCount;

impl Migration for CreateChunkRefCount {
    fn next(&self) -> Option<Box<dyn Migration>> {
        None
    }

    fn up(&self) -> Result<(), crate::Error> {
        crate::vacuum::fix_chunk_ref_count()?;
        Ok(())
    }
}
