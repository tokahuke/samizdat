//! Migrations to be run to evolve the schema of the database and ensure forward
//! version compatibility.

use samizdat_common::{
    Key,
    db::{Migration, Table as _, WritableTx, readonly_tx},
};
use serde_derive::{Deserialize, Serialize};

use super::Table;
use crate::models::SubscriptionKind;

/// Base migration that serves as the starting point for the migration chain
#[derive(Debug)]
pub struct BaseMigration;

impl Migration<Table> for BaseMigration {
    fn next(&self) -> Option<Box<dyn Migration<Table>>> {
        Some(Box::new(CreateChunkRefCount))
    }

    fn up(&self, _: &mut WritableTx) -> Result<(), crate::Error> {
        Ok(())
    }
}

/// Migration to create and initialize the chunk reference counting system
#[derive(Debug)]
struct CreateChunkRefCount;

impl Migration<Table> for CreateChunkRefCount {
    fn next(&self) -> Option<Box<dyn Migration<Table>>> {
        Some(Box::new(BackfillSubscriptionMaxBytes))
    }

    fn up(&self, tx: &mut WritableTx) -> Result<(), crate::Error> {
        crate::vacuum::fix_chunk_ref_count(tx)?;
        Ok(())
    }
}

/// `Subscription` grew a `max_bytes: Option<u64>` field. Bincode wire
/// is positional; existing rows fail to deserialize against the new
/// shape. This migration rewrites each row through a `Legacy` shim
/// matching the old two-field layout and serializes with
/// `max_bytes = None` so the operator default applies.
#[derive(Debug)]
struct BackfillSubscriptionMaxBytes;

#[derive(Debug, Deserialize)]
struct LegacySubscription {
    public_key: Key,
    kind: SubscriptionKind,
}

#[derive(Debug, Serialize)]
struct NewSubscription {
    public_key: Key,
    kind: SubscriptionKind,
    max_bytes: Option<u64>,
}

impl Migration<Table> for BackfillSubscriptionMaxBytes {
    fn next(&self) -> Option<Box<dyn Migration<Table>>> {
        None
    }

    fn up(&self, tx: &mut WritableTx) -> Result<(), crate::Error> {
        let entries: Vec<(Vec<u8>, Vec<u8>)> = readonly_tx(|read_tx| {
            Table::Subscriptions
                .range::<_, [u8; 0]>(..)
                .collect(read_tx, |key, value| {
                    Ok::<_, samizdat_common::Error>((key.to_vec(), value.to_vec()))
                })
                .and_then(|res| res)
        })?;
        for (key, value) in entries {
            let legacy: LegacySubscription = match bincode::deserialize(&value) {
                Ok(legacy) => legacy,
                Err(_) => {
                    // Already in the new shape from a fresh install or
                    // a prior partial run; nothing to do for this row.
                    continue;
                }
            };
            let migrated = NewSubscription {
                public_key: legacy.public_key,
                kind: legacy.kind,
                max_bytes: None,
            };
            Table::Subscriptions.put(
                tx,
                &key,
                bincode::serialize(&migrated).expect("can serialize"),
            )?;
        }
        Ok(())
    }
}
