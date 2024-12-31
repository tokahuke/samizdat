//! Migrations to be run to evolve the schema of the database and ensure forward
//! version compatibility.

use samizdat_common::db::Migration;

use super::Table;

#[derive(Debug)]
pub struct BaseMigration;

impl Migration<Table> for BaseMigration {
    fn next(&self) -> Option<Box<dyn Migration<Table>>> {
        Some(Box::new(CreateChunkRefCount))
    }

    fn up(&self) -> Result<(), crate::Error> {
        Ok(())
    }
}

#[derive(Debug)]
struct CreateChunkRefCount;

impl Migration<Table> for CreateChunkRefCount {
    fn next(&self) -> Option<Box<dyn Migration<Table>>> {
        None
    }

    fn up(&self) -> Result<(), crate::Error> {
        crate::vacuum::fix_chunk_ref_count()?;
        Ok(())
    }
}
