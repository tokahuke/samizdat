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

/// A unique identifier generated from the current time in microseconds.
/// Guaranteed to be monotonically increasing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Id(u64);

impl Id {
    pub const MIN: Id = Id(0);
    pub const MAX: Id = Id(u64::MAX);

    /// Creates a new unique Id based on the current time.
    pub fn generate() -> Self {
        Id(generate_id())
    }

    /// Converts the Id to a byte array. If `desc` is true, the bytes are inverted.
    const fn to_bytes(self, desc: bool) -> [u8; 8] {
        if desc {
            (self.0 ^ u64::MAX).to_be_bytes()
        } else {
            self.0.to_le_bytes()
        }
    }
}

/// Generates an id from the current time in nanoseconds. Ids will always increase
/// monotonically.
fn generate_id() -> u64 {
    let raw_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_micros() as u64;

    static LAST_ID: StdMutex<u64> = StdMutex::new(0);

    let mut last_id = LAST_ID.lock().unwrap();

    if raw_id == *last_id {
        *last_id = raw_id + 1;
    } else {
        *last_id = raw_id;
    }

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
            );
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
