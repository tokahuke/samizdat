//! Migrations to be run to evolve the schema of the database and ensure forward
//! version compatibility.

use std::fmt::Debug;

use super::Table;

/// The `&mut DB` guarantees exclusive access to the db, since this type is not clonable.
pub(super) fn migrate(db: &mut rocksdb::DB) -> Result<(), crate::Error> {
    BaseMigration.migrate(db)
}

trait Migration: Debug {
    fn next(&self) -> Option<Box<dyn Migration>>;
    fn up(&self, db: &mut rocksdb::DB) -> Result<(), crate::Error>;

    fn is_up(&self, db: &rocksdb::DB) -> Result<bool, crate::Error> {
        let migration_key = format!("{self:?}");
        let value = db.get_cf(Table::Migrations.get(), migration_key.as_bytes())?;
        Ok(value.is_some())
    }

    fn migrate(&self, db: &mut rocksdb::DB) -> Result<(), crate::Error> {
        if !self.is_up(db)? {
            let migration_key = format!("{self:?}");

            // This should be atomic, but... oh! dear...
            log::info!("Applying migration {self:?}...");
            self.up(db)?;
            db.put_cf(Table::Migrations.get(), migration_key.as_bytes(), [])?;
            log::info!("... done.");
        } else {
            log::info!("Migration {self:?} already up.");
        }

        // Tail-recurse:
        if let Some(last) = self.next() {
            last.migrate(db)?;
        }

        Ok(())
    }
}

#[derive(Debug)]
struct BaseMigration;

impl Migration for BaseMigration {
    fn next(&self) -> Option<Box<dyn Migration>> {
        None
    }

    fn up(&self, _db: &mut rocksdb::DB) -> Result<(), crate::Error> {
        Ok(())
    }
}
