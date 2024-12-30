//! Migrations to be run to evolve the schema of the database and ensure forward
//! version compatibility.

use std::fmt::Debug;

use super::Table;

/// The `&mut DB` guarantees exclusive access to the db, since this type is not clonable.
pub(super) fn migrate() -> Result<(), crate::Error> {
    BaseMigration.migrate()
}

/// A migration to be run in the database at process start.
trait Migration: Debug {
    fn next(&self) -> Option<Box<dyn Migration>>;
    fn up(&self) -> Result<(), crate::Error>;

    fn is_up(&self) -> bool {
        let migration_key = format!("{self:?}");
        Table::Migrations.atomic_has(migration_key)
    }

    fn migrate(&self) -> Result<(), crate::Error> {
        if !self.is_up() {
            let migration_key = format!("{self:?}");

            // This should be atomic, but... oh! dear...
            tracing::info!("Applying migration {self:?}...");
            self.up()?;
            Table::Migrations.atomic_put(migration_key, []);
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
