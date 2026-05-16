//! Models for storing and managing data in the hub.
//!
//! This module contains the data structures and traits used to represent and manipulate
//! various types of information tracked by the hub.

mod blacklisted_ip;
mod candidate_log;
mod connection_log;
mod query_log;
mod statistics_log;

pub use blacklisted_ip::BlacklistedIp;
pub use candidate_log::CandidateLog;
pub use connection_log::ConnectionLog;
pub use query_log::QueryLog;
pub use statistics_log::StatisticsLog;

use samizdat_common::db::Table as _;
use samizdat_common::db::TableRange;
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};
use serde_derive::{Deserialize, Serialize};
use std::ops::Range;
use std::sync::Mutex as StdMutex;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use samizdat_common::db::writable_tx;

use crate::db::Table;

/// A unique identifier generated from the current time in microseconds. Guaranteed
/// to be monotonically increasing across calls within a single process; see
/// [`generate_id`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Id(u64);

impl Id {
    pub const MIN: Id = Id(0);
    pub const MAX: Id = Id(u64::MAX);

    /// Creates a new unique Id based on the current time.
    pub fn generate() -> Self {
        Id(generate_id())
    }

    /// Converts the Id to a byte array. Both branches use big-endian so the bytes
    /// sort lexicographically (LMDB orders keys bytewise). Previously the `!desc`
    /// branch used little-endian, which would have silently broken range scans on
    /// any table that opted in to ascending order; today every table defaults to
    /// `DESC = true` so it was latent, but the encoding has to be consistent.
    const fn to_bytes(self, desc: bool) -> [u8; 8] {
        if desc {
            (self.0 ^ u64::MAX).to_be_bytes()
        } else {
            self.0.to_be_bytes()
        }
    }
}

/// Monotonic id allocator. Lives at module scope so its purpose is discoverable;
/// every `*Log` table primary key comes through `generate_id` and so through this
/// counter.
static LAST_ID: StdMutex<u64> = StdMutex::new(0);

/// Generates a strictly monotonic id derived from the current wall clock.
///
/// Resolution is microseconds. Two calls in the same microsecond, or any backward
/// clock jump (NTP correction, leap-second handling, VM clock skew), would otherwise
/// produce duplicate ids and overwrite each other in `ConnectionLog` / `QueryLog` /
/// `CandidateLog` tables. The `max(last + 1, raw)` keeps the sequence strictly
/// increasing across both cases; the cost is that after a backward jump the ids
/// drift slightly ahead of wall clock until time catches up.
///
/// If `SystemTime::now()` somehow predates the Unix epoch (a pathological clock
/// state, but observed in VMs that boot with the RTC unset), we fall back to
/// `Duration::ZERO` rather than panicking; `max(last + 1, 0)` still advances.
fn generate_id() -> u64 {
    let raw_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64;

    let mut last_id = LAST_ID.lock().unwrap();
    *last_id = last_id.saturating_add(1).max(raw_id);
    *last_id
}

/// A trait for types that can be stored in the database with a unique identifier.
///
/// This trait provides functionality for serializing and storing objects in the database,
/// with each object having a unique ID that can be used for retrieval.
pub trait Indexable: SerdeSerialize + for<'a> SerdeDeserialize<'a> {
    /// The database table where instances of this type are stored.
    const TABLE: Table;

    /// Whether to sort the table in descending order.
    const DESC: bool = true;

    /// Returns the unique identifier for this instance.
    fn id(&self) -> Id;

    /// Inserts this instance into its associated database table.
    ///
    /// # Returns
    /// The ID of the inserted item.
    fn insert(&self) -> Id {
        let id = self.id();
        writable_tx(|tx| {
            Self::TABLE.put(
                tx,
                id.to_bytes(Self::DESC),
                bincode::serialize(self).expect("can serialize"),
            )?;
            Ok(())
        })
        .expect("infallible");

        id
    }

    fn range(from: Id, to: Id) -> TableRange<Table, Range<[u8; 8]>, [u8; 8]> {
        if Self::DESC {
            Self::TABLE.range(to.to_bytes(true)..from.to_bytes(true))
        } else {
            Self::TABLE.range(from.to_bytes(false)..to.to_bytes(false))
        }
    }

    // For future use:
    // /// Updates the item with the given id.
    // fn update<F>(id: Id, f: F)
    // where
    //     F: FnOnce(&mut Self),
    // {
    //     writable_tx(|tx| {
    //         let mut item = Self::TABLE.get(
    //             tx,
    //             bincode::serialize(&id).expect("can serialize"),
    //             |bytes| bincode::deserialize(bytes).expect("can deserialize"),
    //         );

    //         if let Some(item) = &mut item {
    //             f(item);

    //             Self::TABLE.put(
    //                 tx,
    //                 bincode::serialize(&id).expect("can serialize"),
    //                 bincode::serialize(item).expect("can serialize"),
    //             );
    //         }

    //         Ok(())
    //     })
    //     .expect("infallible");
    // }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for H8. `generate_id` must be strictly monotonic across
    /// rapid back-to-back calls (the same-microsecond case) and must not produce
    /// duplicates on a backward clock jump. The old code did
    /// `*last_id = raw_id` even when `raw_id < *last_id`, which silently
    /// produced collisions in `*Log` tables.
    #[test]
    fn generate_id_is_strictly_monotonic_in_burst() {
        let n = 10_000;
        let mut ids = Vec::with_capacity(n);
        for _ in 0..n {
            ids.push(generate_id());
        }
        for w in ids.windows(2) {
            assert!(w[0] < w[1], "ids not strictly increasing: {} >= {}", w[0], w[1]);
        }
    }

    /// Regression test for H5. Both ascending and descending encodings must
    /// preserve lexicographic order over the underlying integer order. LMDB
    /// orders keys bytewise; a `to_le_bytes` encoding (the previous bug)
    /// would sort `0x00000100` before `0x00000080`.
    #[test]
    fn to_bytes_preserves_order_both_directions() {
        // Mix small, medium, and large values so byte-wise comparison doesn't
        // accidentally pass on a small-magnitude range. `wrapping_mul` so the
        // test itself doesn't overflow on release builds.
        let ids: Vec<Id> = (0..1000u64)
            .map(|i| Id(i.wrapping_mul(0x1234_5678_9abc_def0)))
            .collect();

        // Ascending: lexicographic order matches integer order.
        let mut asc: Vec<[u8; 8]> = ids.iter().map(|i| i.to_bytes(false)).collect();
        asc.sort();
        let asc_ids: Vec<Id> = asc
            .iter()
            .map(|b| Id(u64::from_be_bytes(*b)))
            .collect();
        let mut expected = ids.clone();
        expected.sort();
        assert_eq!(asc_ids, expected, "ascending bytes did not sort by id");

        // Descending: lexicographic order is reverse of integer order.
        let mut desc: Vec<[u8; 8]> = ids.iter().map(|i| i.to_bytes(true)).collect();
        desc.sort();
        let desc_ids: Vec<Id> = desc
            .iter()
            .map(|b| Id(u64::from_be_bytes(*b) ^ u64::MAX))
            .collect();
        let mut expected_desc = ids;
        expected_desc.sort_by(|a, b| b.cmp(a));
        assert_eq!(desc_ids, expected_desc, "descending bytes did not sort by reversed id");
    }
}
