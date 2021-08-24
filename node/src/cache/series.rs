use ed25519_dalek::{Keypair, PublicKey};
use rocksdb::IteratorMode;
use serde_derive::{Deserialize, Serialize};
use std::fmt::{self, Display};
use std::time::Duration;

use samizdat_common::pki::Signed;

use crate::db;
use crate::db::Table;

use super::CollectionRef;

#[derive(Debug, Serialize, Deserialize)]
pub struct SeriesOwner {
    keypair: Keypair,
    default_ttl: Duration,
}

impl SeriesOwner {
    pub fn create(name: &str, default_ttl: Duration) -> Result<SeriesOwner, crate::Error> {
        let owner = SeriesOwner {
            keypair: Keypair::generate(&mut samizdat_common::csprng()),
            default_ttl,
        };
        let series = owner.series();

        let mut batch = rocksdb::WriteBatch::default();

        batch.put_cf(
            Table::SeriesOwners.get(),
            name.as_bytes(),
            bincode::serialize(&owner).expect("can serialize"),
        );
        batch.put_cf(
            Table::Series.get(),
            series.key(),
            bincode::serialize(&series).expect("can serialize"),
        );

        db().write(batch)?;

        Ok(owner)
    }

    pub fn get(name: &str) -> Result<Option<SeriesOwner>, crate::Error> {
        let maybe_serialized = db().get_cf(Table::SeriesOwners.get(), name.as_bytes())?;
        if let Some(serialized) = maybe_serialized {
            let owner = bincode::deserialize(&serialized)?;
            Ok(Some(owner))
        } else {
            Ok(None)
        }
    }

    pub fn series(&self) -> SeriesRef {
        SeriesRef {
            public_key: self.keypair.public,
        }
    }

    fn sign(&self, collection: CollectionRef) -> SeriesItem {
        SeriesItem {
            signed: Signed::new(
                SeriesItemContent {
                    collection,
                    timestamp: chrono::Utc::now(),
                    ttl: self.default_ttl,
                },
                &self.keypair,
            ),
            public_key: self.keypair.public,
            freshness: chrono::Utc::now(),
        }
    }

    pub fn advance(&self, collection: CollectionRef) -> Result<SeriesItem, crate::Error> {
        let item = self.sign(collection);

        db().put_cf(
            Table::SeriesItems.get(),
            item.key(),
            bincode::serialize(&item).expect("can serialize"),
        )?;

        Ok(item)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SeriesRef {
    public_key: PublicKey,
}

impl Display for SeriesRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", base64_url::encode(self.key()),)
    }
}

impl SeriesRef {
    pub fn new(public_key: PublicKey) -> SeriesRef {
        SeriesRef { public_key }
    }

    pub fn key(&self) -> &[u8] {
        self.public_key.as_bytes()
    }

    pub fn is_locally_owned(&self) -> Result<bool, crate::Error> {
        // TODO: make this not a SeqScan, perhaps?
        for (_, owner) in db().iterator_cf(Table::SeriesOwners.get(), IteratorMode::Start) {
            let owner: SeriesOwner = bincode::deserialize(&owner)?;
            if self.public_key == owner.keypair.public {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Returns the latest fresh collection in the local database _or_ the latest collection if the
    /// series is locally owned.
    pub fn get_latest_fresh(&self) -> Result<Option<CollectionRef>, crate::Error> {
        let is_locally_owned = self.is_locally_owned()?;

        // TODO: use prefix trickery to avoid mini-SeqScan here.
        let iter = db().prefix_iterator_cf(Table::SeriesItems.get(), &self.public_key);
        let mut max: Option<SeriesItem> = None;

        for (_key, value) in iter {
            let item: SeriesItem = bincode::deserialize(&value)?;

            if !item.is_fresh() && !is_locally_owned {
                continue;
            }

            if let Some(max) = max.as_mut() {
                if max.signed.timestamp < item.signed.timestamp {
                    *max = item;
                }
            } else {
                max = Some(item);
            }
        }

        Ok(max.map(|max| max.collection()))
    }

    pub fn advance(&self, series_item: SeriesItem) -> Result<(), crate::Error> {
        if !series_item.is_valid() {
            return Err(crate::Error::InvalidSeriesItem);
        }

        if self.public_key != series_item.public_key {
            return Err(crate::Error::DifferentePublicKeys);
        }

        db().put_cf(
            Table::SeriesItems.get(),
            series_item.key(),
            bincode::serialize(&series_item).expect("can serialize"),
        )?;

        // TODO: do some cleanup on the old values.

        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SeriesItemContent {
    collection: CollectionRef,
    timestamp: chrono::DateTime<chrono::Utc>,
    ttl: Duration,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SeriesItem {
    signed: Signed<SeriesItemContent>,
    public_key: PublicKey,
    /// Remember to clear this field when sending to the wire.
    freshness: chrono::DateTime<chrono::Utc>,
}

impl SeriesItem {
    pub fn collection(&self) -> CollectionRef {
        self.signed.collection.clone()
    }

    pub fn is_valid(&self) -> bool {
        self.signed.verify(&self.public_key)
    }

    pub fn public_key(&self) -> &PublicKey {
        &self.public_key
    }

    #[inline(always)]
    fn key(&self) -> Vec<u8> {
        let timestamp = self.signed.timestamp.timestamp_millis();
        [
            self.public_key.as_bytes().as_ref(),
            timestamp.to_be_bytes().as_ref(),
        ]
        .concat()
    }

    pub fn erase_freshness(&mut self) {
        self.freshness =
            chrono::DateTime::from_utc(chrono::NaiveDateTime::from_timestamp(0, 0), chrono::Utc);
    }

    pub fn make_fresh(&mut self) {
        self.freshness = chrono::Utc::now();
    }

    pub fn is_fresh(&self) -> bool {
        self.freshness
            + chrono::Duration::from_std(self.signed.ttl).expect("can convert from std duration")
            < chrono::Utc::now()
    }
}

#[test]
fn generate_series_ownership() {
    println!(
        "{}",
        SeriesOwner::create("a series", Duration::from_secs(3600))
            .unwrap()
            .series()
    );
}

#[test]
fn validate_series_item() {
    let owner = SeriesOwner::create("a series", Duration::from_secs(3600)).unwrap();
    let _series = owner.series();

    let current_collection = CollectionRef::rand();

    let series_item = owner.sign(current_collection);

    assert!(series_item.is_valid())
}

#[test]
fn not_validate_series_item() {
    let owner = SeriesOwner::create("a series", Duration::from_secs(3600)).unwrap();

    let other_owner = SeriesOwner::create("another series", Duration::from_secs(3600)).unwrap();
    let other_series = other_owner.series();

    let current_collection = CollectionRef::rand();

    let mut series_item = owner.sign(current_collection);
    series_item.public_key = other_series.public_key;

    assert!(!series_item.is_valid())
}
