//! Migrations to be run to evolve the schema of the database and ensure forward
//! version compatibility.

use std::fmt::Debug;

use samizdat_common::db::{Migration, WritableTx};

use super::Table;

#[derive(Debug)]
pub(super) struct BaseMigration;

impl Migration<Table> for BaseMigration {
    fn next(&self) -> Option<Box<dyn Migration<Table>>> {
        None
    }

    fn up(&self, _: &mut WritableTx<'_>) -> Result<(), crate::Error> {
        Ok(())
    }
}
