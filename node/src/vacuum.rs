//! A process to keep the size of the database under control and to purge junk
//! that is not used anymore.

use decorum::NotNan;
use rocksdb::{IteratorMode, WriteBatch};
use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap, VecDeque};
use std::time::Duration;
use tokio::runtime::Handle;
use tokio::time::{sleep, Instant};
use serde_derive::{Serialize, Deserialize};

use samizdat_common::heap_entry::HeapEntry;
use samizdat_common::Hash;

use crate::cli::cli;
use crate::db::{db, Table};
use crate::models::{CollectionItem, Dropable, ObjectRef, ObjectStatistics, UsePrior};

/// Status for a vacuum task.
#[derive(Debug, Serialize, Deserialize)]
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
    // Do the vacuum operation atomically to avoid mishaps (resource leakage):
    let mut batch = WriteBatch::default();

    // Test whether you should vacuum:
    let mut total_size = 0;
    for (_, value) in db().iterator_cf(Table::ObjectStatistics.get(), IteratorMode::Start) {
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
    let use_prior = UsePrior {
        gamma_alpha: 1.,
        gamma_beta: 86400., // one day in secs
        beta_alpha: 1.,
        beta_beta: 1.,
    };

    // Test what is good and what isn't:
    for (key, value) in db().iterator_cf(Table::ObjectStatistics.get(), IteratorMode::Start) {
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
    for (_, value) in db().iterator_cf(Table::CollectionItems.get(), IteratorMode::Start) {
        let item: CollectionItem = bincode::deserialize(&value)?;
        if dropped.contains(item.inclusion_proof.claimed_value()) {
            item.drop_if_exists_with(&mut batch)?;
        }
    }

    // Apply all changes atomically:
    db().write(batch)?;

    Ok(status)
}

/// Runs vacuum tasks forever.
pub async fn run_vacuum_daemon() {
    const TIMING_BUFFER_SIZE: usize = 7;
    const VACUUM_TIMESHARE: f64 = 0.05;
    const MIN_INTERLUDE: Duration = Duration::from_secs(2);

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
