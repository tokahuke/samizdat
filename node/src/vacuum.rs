use rocksdb::IteratorMode;
use std::collections::BinaryHeap;
use decorum::NotNan;
use std::cmp::Reverse;

use samizdat_common::Hash;
use samizdat_common::heap_entry::HeapEntry;

use crate::models::{ObjectStatistics, ObjectRef};
use crate::db::{db, Table};
use crate::cli::cli;

pub enum VacuumStatus {
    /// Storage is within allowed parameters.
    Unnecessary,
    /// Removed all disposable content, but could not achieve the desired maximum size.
    Insufficient,
    /// Storage has run and was able to reduce the storage size.
    Done,
}

pub fn vacuum() -> Result<VacuumStatus, crate::Error> {
    let mut total_size = 0;
    for (_, value) in db().iterator_cf(Table::ObjectStatistics.get(), IteratorMode::Start) {
        let statistics: ObjectStatistics = bincode::deserialize(&value)?;
        total_size += statistics.size();
    }
    
    // If within limits, very ok!
    if total_size < cli().max_storage {
        return Ok(VacuumStatus::Unnecessary);
    }

    let mut heap = BinaryHeap::new();

    for (key, value) in db().iterator_cf(Table::ObjectStatistics.get(), IteratorMode::Start) {
        let statistics: ObjectStatistics = bincode::deserialize(&value)?;
        heap.push(HeapEntry { 
            priority: Reverse(NotNan::from(statistics.byte_usefulness())),
            content: (key, statistics.size()),
        });
    }

    while total_size > cli().max_storage {
        if let Some(HeapEntry { content: (key, size), .. }) = heap.pop() {
            let object = ObjectRef::new(Hash::new(key));
            object.drop_if_exists()?;
            total_size -= size;
        } else {
            return Ok(VacuumStatus::Insufficient);
        }
    }

    Ok(VacuumStatus::Done)
}
