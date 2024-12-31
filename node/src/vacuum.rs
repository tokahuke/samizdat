//! A process to keep the size of the database under control and to purge junk
//! that is not used anymore.

use ordered_float::NotNan;
use samizdat_common::db::{writable_tx, Droppable, Table as _};
use serde_derive::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::time::{sleep, Instant};

use samizdat_common::heap_entry::HeapEntry;
use samizdat_common::Hash;

use crate::cli::cli;
use crate::db::{MergeOperation, Table};
use crate::models::{CollectionItem, ObjectMetadata, ObjectRef, ObjectStatistics, UsePrior};

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

    // Test whether you should vacuum:
    let mut total_size = 0;
    Table::ObjectStatistics
        .range(..)
        .atomic_for_each(|_, statistics| {
            total_size += bincode::deserialize::<ObjectStatistics>(statistics)
                .expect("can deserialize")
                .size();
            None as Option<()>
        });

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
    Table::ObjectStatistics
        .range(..)
        .atomic_for_each(|key, value| {
            let statistics: ObjectStatistics =
                bincode::deserialize(value).expect("can deserialize");
            heap.push(HeapEntry {
                priority: Reverse(
                    NotNan::try_from(statistics.byte_usefulness(&use_prior))
                        .expect("byte usefulness was nan"),
                ),
                content: (key.to_vec(), statistics.size()),
            });

            None as Option<()>
        });

    // Prune until you get under the threshold.
    let mut status = VacuumStatus::Done;
    let mut dropped = BTreeSet::new();

    writable_tx(|tx| {
        while total_size >= cli().max_storage * 1_000_000 {
            if let Some(HeapEntry {
                content: (key, size),
                ..
            }) = heap.pop()
            {
                let object = ObjectRef::new(Hash::new(key));
                if !object.is_bookmarked()? {
                    object.drop_if_exists_with(tx)?;
                    dropped.insert(*object.hash());
                    total_size -= size;
                }
            } else {
                status = VacuumStatus::Insufficient;
                break;
            }
        }

        tracing::debug!("to drop: {:#?}", dropped);

        // Prune items:
        let mut items_to_drop = vec![];

        Table::CollectionItems.range(..).for_each(tx, |_, value| {
            let item: CollectionItem = bincode::deserialize(value).expect("can deserialize");
            if dropped.contains(item.inclusion_proof.claimed_value()) {
                items_to_drop.push(item);
            }

            None as Option<()>
        });

        for item in items_to_drop {
            item.drop_if_exists_with(tx)?;
        }

        Ok(())
    })?;

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
            tracing::debug!("vacuum task started");

            match vacuum() {
                Ok(VacuumStatus::Unnecessary | VacuumStatus::Done) => {}
                Ok(VacuumStatus::Insufficient) => {
                    tracing::warn!("vacuum task was insufficient to bring storage size down")
                }
                Err(err) => tracing::error!("vacuum task error: {}", err),
            }

            tracing::debug!("vacuum task ended");
        });

        if let Err(err) = vacuum_task.await {
            tracing::error!("vacuum task panicked: {}", err);
        }

        let end = Instant::now();

        tracing::debug!("vacuum task took {:?}", end - start);

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
    let mut all_objects = vec![];

    Table::ObjectMetadata.range(..).atomic_for_each(|hash, _| {
        all_objects.push(ObjectRef::new(Hash::new(hash)));
        None as Option<()>
    });

    // No transaction here! Might take too long. Better to break in smaller
    // one-per-object chunks.
    for object in all_objects {
        if let Err(err) = object.drop_if_exists() {
            tracing::warn!("Failed to drop {object:?}: {err}");
        }
    }
}

/// Fixes chunk reference count.
pub fn fix_chunk_ref_count() -> Result<(), crate::Error> {
    let mut ref_counts = BTreeMap::new();

    Table::ObjectMetadata
        .range(..)
        .atomic_for_each(|_, metadata| {
            let metadata: ObjectMetadata =
                bincode::deserialize(metadata).expect("can deserialize");

            for chunk_hash in metadata.hashes {
                *ref_counts.entry(chunk_hash).or_default() += 1;
            }

            None as Option<()>
        });

    writable_tx(|tx| {
        for (hash, ref_count) in ref_counts {
            Table::ObjectChunkRefCount.map(
                tx,
                hash,
                MergeOperation::Set(ref_count).merger(),
            );
        }

        Ok(())
    })
}

/// Drop chunks not associated with any object, i.e., those where the reference count has
/// dropped to zero.
///
/// # Note:
///
/// Only call this function in a __blocking__ context. If `async` is needed, refactor!
fn drop_orphan_chunks() -> Result<usize, crate::Error> {
    let mut chunks_to_drop = vec![];

    Table::ObjectChunkRefCount
        .range(..)
        .atomic_for_each(|hash, ref_count| {
            let hash = Hash::new(hash);
            let ref_count: MergeOperation =
                bincode::deserialize(ref_count).expect("can deserialize");

            match ref_count.eval_on_zero() {
                1.. => {}
                0 => chunks_to_drop.push(hash),
                neg => tracing::error!("Chunk {hash} reference count dropped to negative: {neg}!"),
            }

            None as Option<()>
        });

    writable_tx(|tx| {
        let dropped = chunks_to_drop.len();

        for hash in chunks_to_drop {
            Table::ObjectChunks.delete(tx, hash);
        }

        Ok(dropped)
    })
}

/// Drop items that don't point to anything anymore.
fn drop_dangling_items() -> Result<usize, crate::Error> {
    let mut items_to_drop = vec![];

    let outcome = Table::CollectionItems.range(..).atomic_for_each(|_, item| {
        let item: CollectionItem = bincode::deserialize(item).expect("can deserialize");

        item.object()
            .and_then(|o| o.exists())
            .map(|exists| {
                if !exists {
                    items_to_drop.push(item);
                }
            })
            .err()?;

        None
    });

    if let Some(err) = outcome {
        return Err(err);
    }

    writable_tx(|tx| {
        let dropped = items_to_drop.len();

        for item in items_to_drop {
            item.drop_if_exists_with(tx)?;
        }

        Ok(dropped)
    })
}
