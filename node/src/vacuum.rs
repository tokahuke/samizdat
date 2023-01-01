//! A process to keep the size of the database under control and to purge junk
//! that is not used anymore.

use decorum::NotNan;
use rocksdb::{IteratorMode, WriteBatch};
use serde_derive::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::time::{sleep, Instant};

use samizdat_common::heap_entry::HeapEntry;
use samizdat_common::Hash;

use crate::cli::cli;
use crate::db::{db, MergeOperation, Table, CHUNK_RW_LOCK};
use crate::models::{
    CollectionItem, Droppable, ObjectMetadata, ObjectRef, ObjectStatistics, UsePrior,
};

/// Status for a vacuum task.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
pub enum VacuumStatus {
    /// Storage is within allowed parameters.
    Unnecessary,
    /// Removed all disposable content, but could not achieve the desired maximum size.
    Insufficient,
    /// Storage has run and was able to reduce the storage size.
    Done,
}

/// Run a vacuum round in the database.
pub fn vacuum() -> Result<VacuumStatus, crate::Error> {
    // STEP 1: make up space if needed, deleting rarely used stuff:

    // Do the vacuum operation atomically to avoid mishaps (resource leakage):
    let mut batch = WriteBatch::default();

    // Test whether you should vacuum:
    let mut total_size = 0;
    for item in db().iterator_cf(Table::ObjectStatistics.get(), IteratorMode::Start) {
        let (_, value) = item?;
        let statistics: ObjectStatistics = bincode::deserialize(&value)?;
        total_size += statistics.size();
    }

    // If within limits, very ok!
    if total_size < cli().max_storage * 1_000_000 {
        return Ok(VacuumStatus::Unnecessary);
    }

    // Else, prune!
    let mut heap = BinaryHeap::new();

    // Define a prior for use:
    // TODO: how to calibrate correctly?
    let use_prior = UsePrior::default();

    // Test what is good and what isn't:
    for item in db().iterator_cf(Table::ObjectStatistics.get(), IteratorMode::Start) {
        let (key, value) = item?;
        let statistics: ObjectStatistics = bincode::deserialize(&value)?;
        heap.push(HeapEntry {
            priority: Reverse(NotNan::from(statistics.byte_usefulness(&use_prior))),
            content: (key, statistics.size()),
        });
    }

    // Prune until you get under the threshold.
    let mut status = VacuumStatus::Done;
    let mut dropped = BTreeSet::new();
    while total_size >= cli().max_storage * 1_000_000 {
        if let Some(HeapEntry {
            content: (key, size),
            ..
        }) = heap.pop()
        {
            let object = ObjectRef::new(Hash::new(key));
            if !object.is_bookmarked()? {
                object.drop_if_exists_with(&mut batch)?;
                dropped.insert(*object.hash());
                total_size -= size;
            }
        } else {
            status = VacuumStatus::Insufficient;
            break;
        }
    }

    log::debug!("to drop: {:#?}", dropped);

    // Prune items:
    for item in db().iterator_cf(Table::CollectionItems.get(), IteratorMode::Start) {
        let (_, value) = item?;
        let item: CollectionItem = bincode::deserialize(&value)?;
        if dropped.contains(item.inclusion_proof.claimed_value()) {
            item.drop_if_exists_with(&mut batch)?;
        }
    }

    // Apply all changes atomically:
    db().write(batch)?;

    // STEP 2: garbage collection:
    let dropped_chunks = drop_orphan_chunks()?;
    let dropped_items = drop_dangling_items()?;

    if (dropped_chunks > 0 || dropped_items > 0) && status == VacuumStatus::Unnecessary {
        status = VacuumStatus::Done;
    }

    Ok(status)
}

/// Runs vacuum tasks forever.
pub async fn run_vacuum_daemon() {
    const TIMING_BUFFER_SIZE: usize = 7;
    const VACUUM_TIMESHARE: f64 = 0.05;
    const MIN_INTERLUDE: Duration = Duration::from_secs(30);

    let mut last_timings = VecDeque::new();
    let mut push_timing = |timing| {
        last_timings.push_back(timing);

        if last_timings.len() > TIMING_BUFFER_SIZE {
            last_timings.pop_front();
        }

        last_timings.iter().copied().sum::<Duration>()
    };

    loop {
        let start = Instant::now();
        let vacuum_task = Handle::current().spawn_blocking(|| {
            log::debug!("vacuum task started");

            match vacuum() {
                Ok(VacuumStatus::Unnecessary | VacuumStatus::Done) => {}
                Ok(VacuumStatus::Insufficient) => {
                    log::warn!("vacuum task was insufficient to bring storage size down")
                }
                Err(err) => log::error!("vacuum task error: {}", err),
            }

            log::debug!("vacuum task ended");
        });

        if let Err(err) = vacuum_task.await {
            log::error!("vacuum task panicked: {}", err);
        }

        let end = Instant::now();

        log::debug!("vacuum task took {:?}", end - start);

        let moving_avg_timing = push_timing(end - start);
        let interlude = moving_avg_timing.mul_f64(1. / VACUUM_TIMESHARE - 1.);

        sleep(if interlude > MIN_INTERLUDE {
            interlude
        } else {
            MIN_INTERLUDE
        })
        .await;
    }
}

/// Flushes the whole local cash.
pub fn flush_all() {
    // This is slow and inefficient, but at least it will be correct.
    for item in db().iterator_cf(Table::ObjectMetadata.get(), IteratorMode::Start) {
        match item {
            Ok((hash, _)) => {
                let object = ObjectRef::new(Hash::new(hash));
                if let Err(err) = object.drop_if_exists() {
                    log::warn!("Failed to drop {object:?}: {err}");
                }
            }
            Err(err) => log::warn!("Failed to load an object from db for deletion: {err}"),
        }
    }
}

/// Fixes chunk reference count.
pub fn fix_chunk_ref_count() -> Result<(), crate::Error> {
    let mut ref_counts = BTreeMap::new();

    for item in db().iterator_cf(Table::ObjectMetadata.get(), IteratorMode::Start) {
        let (_, metadata) = item?;
        let metadata: ObjectMetadata = bincode::deserialize(&metadata)?;
        for chunk_hash in metadata.hashes {
            *ref_counts.entry(chunk_hash).or_default() += 1;
        }
    }

    let mut batch = rocksdb::WriteBatch::default();

    for (hash, ref_count) in ref_counts {
        batch.merge_cf(
            Table::ObjectChunkRefCount.get(),
            &hash,
            bincode::serialize(&MergeOperation::Set(ref_count)).expect("can serialize"),
        );
    }

    db().write(batch)?;

    Ok(())
}

/// Drop chunks not associated with any object, i.e., those where the reference count has
/// dropped to zero.
///
/// # Note:
///
/// Only call this function in a __blocking__ context. If `async` is needed, refactor!
fn drop_orphan_chunks() -> Result<usize, crate::Error> {
    // This is only called in a blocking context:
    let chunk_lock = CHUNK_RW_LOCK.blocking_write();
    let mut batch = rocksdb::WriteBatch::default();

    for item in db().iterator_cf(Table::ObjectChunkRefCount.get(), IteratorMode::Start) {
        let (hash, ref_count) = item?;
        let hash = Hash::new(hash);
        let ref_count: MergeOperation = bincode::deserialize(&ref_count)?;

        match ref_count.eval_on_zero() {
            1.. => {}
            0 => batch.delete_cf(Table::ObjectChunks.get(), &hash),
            neg => log::error!("Chunk {hash} reference count dropped to negative: {neg}!"),
        }
    }

    let dropped = batch.len();

    db().write(batch)?;
    drop(chunk_lock);

    Ok(dropped)
}

/// Drop items that don't point to anything anymore.
fn drop_dangling_items() -> Result<usize, crate::Error> {
    let mut batch = rocksdb::WriteBatch::default();

    for item in db().iterator_cf(Table::CollectionItems.get(), IteratorMode::Start) {
        let (_, item) = item?;
        let item: CollectionItem = bincode::deserialize(&item)?;

        if !item.object()?.exists()? {
            item.drop_if_exists_with(&mut batch)?;
        }
    }

    let dropped = batch.len();
    db().write(batch)?;

    Ok(dropped)
}
