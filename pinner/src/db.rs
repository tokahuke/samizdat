//! Local persistence for the pinner: a single sub-database tracking which
//! series the pinner is keeping alive and until when.
//!
//! Schema is small enough to never need a migration beyond the base; the
//! `Migrations` table is here only because `samizdat_common::db::Table`
//! requires it.

use chrono::{DateTime, Utc};
use samizdat_common::db::{
    init_db, readonly_tx, writable_tx, Migration, Table as _, WritableTx,
};
use samizdat_common::Key;
use serde_derive::{Deserialize, Serialize};
use strum_macros::{IntoStaticStr, VariantArray};

#[derive(Debug, Clone, Copy, IntoStaticStr, VariantArray)]
#[non_exhaustive]
pub enum Table {
    /// Required by `samizdat_common::db::Table` to track applied migrations.
    Migrations,
    /// Maps `Key::as_bytes()` (the series public key) to a bincode-serialized
    /// [`PinnedRow`].
    PinnedSeries,
}

impl samizdat_common::db::Table for Table {
    const MIGRATIONS: Self = Table::Migrations;

    fn base_migration() -> Box<dyn Migration<Self>> {
        Box::new(BaseMigration)
    }

    fn discriminant(self) -> usize {
        self as usize
    }
}

#[derive(Debug)]
struct BaseMigration;

impl Migration<Table> for BaseMigration {
    fn next(&self) -> Option<Box<dyn Migration<Table>>> {
        None
    }

    fn up(&self, _tx: &mut WritableTx) -> Result<(), samizdat_common::Error> {
        Ok(())
    }
}

/// What the pinner records for each subscription it is managing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinnedRow {
    pub expires_at: DateTime<Utc>,
    pub customer: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub fn init(data_dir: &str) -> Result<(), anyhow::Error> {
    init_db::<Table>(data_dir).map_err(|e| anyhow::anyhow!("db init: {e}"))?;
    Ok(())
}

/// Upserts a pin. On re-pin extends `expires_at` to the later of (existing,
/// new) so a customer re-POSTing before expiry renews their slot.
pub fn upsert(
    key: &Key,
    new_expires_at: DateTime<Utc>,
    customer: Option<String>,
) -> Result<DateTime<Utc>, anyhow::Error> {
    writable_tx(|tx| {
        let existing: Option<PinnedRow> =
            Table::PinnedSeries.get(tx, key.as_bytes(), |bytes| Ok(bincode::deserialize(bytes)?))?;

        let row = match existing {
            Some(mut existing) => {
                if new_expires_at > existing.expires_at {
                    existing.expires_at = new_expires_at;
                }
                // Preserve customer + created_at on renewal.
                existing
            }
            None => PinnedRow {
                expires_at: new_expires_at,
                customer,
                created_at: Utc::now(),
            },
        };

        Table::PinnedSeries.put(
            tx,
            key.as_bytes(),
            bincode::serialize(&row).expect("can serialize"),
        )?;

        Ok::<_, samizdat_common::Error>(row.expires_at)
    })
    .map_err(|e| anyhow::anyhow!("db upsert: {e}"))
}

pub fn get(key: &Key) -> Result<Option<PinnedRow>, anyhow::Error> {
    readonly_tx(|tx| {
        Table::PinnedSeries
            .get(tx, key.as_bytes(), |bytes| Ok(bincode::deserialize(bytes)?))
            .map_err(|e| anyhow::anyhow!("db get: {e}"))
    })
}

pub fn list() -> Result<Vec<(Key, PinnedRow)>, anyhow::Error> {
    // Inner result short-circuits on the first per-row failure, outer
    // result reports a transaction-level failure. Two `?`s unwrap both.
    let collected: Result<Vec<(Key, PinnedRow)>, samizdat_common::Error> = readonly_tx(|tx| {
        Table::PinnedSeries
            .range::<_, [u8; 0]>(..)
            .collect(tx, |key, value| -> Result<(Key, PinnedRow), samizdat_common::Error> {
                let parsed_key = Key::from_bytes(key)
                    .map_err(|e| samizdat_common::Error::from(format!("bad key: {e}")))?;
                let row: PinnedRow = bincode::deserialize(value)?;
                Ok((parsed_key, row))
            })
            .and_then(|res| res)
    });
    collected.map_err(|e| anyhow::anyhow!("db list: {e}"))
}

pub fn delete(key: &Key) -> Result<(), anyhow::Error> {
    writable_tx(|tx| {
        Table::PinnedSeries.delete(tx, key.as_bytes())?;
        Ok::<_, samizdat_common::Error>(())
    })
    .map_err(|e| anyhow::anyhow!("db delete: {e}"))
}

/// Returns keys + rows whose `expires_at` is in the past, for the expiry loop.
pub fn list_expired(now: DateTime<Utc>) -> Result<Vec<Key>, anyhow::Error> {
    let all = list()?;
    Ok(all
        .into_iter()
        .filter(|(_, row)| row.expires_at <= now)
        .map(|(key, _)| key)
        .collect())
}

