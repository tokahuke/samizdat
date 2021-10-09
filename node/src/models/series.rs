use chrono::SubsecRound;
use ed25519_dalek::Keypair;
use rocksdb::{IteratorMode, WriteBatch};
use serde_derive::{Deserialize, Serialize};
use std::fmt::{self, Display};
use std::time::Duration;

use samizdat_common::{ContentRiddle, Key, PrivateKey, Signed};

use crate::db;
use crate::db::Table;

use super::{BookmarkType, CollectionRef, Dropable};

#[derive(Debug, Serialize, Deserialize)]
pub struct SeriesOwner {
    name: String,
    keypair: Keypair,
    default_ttl: Duration,
    #[serde(default)]
    is_draft: bool,
}

impl Dropable for SeriesOwner {
    fn drop_if_exists_with(&self, batch: &mut WriteBatch) -> Result<(), crate::Error> {
        self.series().drop_if_exists_with(batch)?;
        batch.delete_cf(Table::SeriesOwners.get(), self.name.as_bytes());

        Ok(())
    }
}

impl SeriesOwner {
    fn insert(&self, batch: &mut WriteBatch) {
        let series = self.series();

        batch.put_cf(
            Table::SeriesOwners.get(),
            self.name.as_bytes(),
            bincode::serialize(&self).expect("can serialize"),
        );
        batch.put_cf(
            Table::Series.get(),
            series.key(),
            bincode::serialize(&series).expect("can serialize"),
        );
    }

    pub fn create(
        name: &str,
        default_ttl: Duration,
        is_draft: bool,
    ) -> Result<SeriesOwner, crate::Error> {
        let owner = SeriesOwner {
            name: name.to_owned(),
            keypair: Keypair::generate(&mut samizdat_common::csprng()),
            default_ttl,
            is_draft,
        };

        let mut batch = WriteBatch::default();

        owner.insert(&mut batch);

        db().write(batch)?;

        Ok(owner)
    }

    pub fn import(
        name: &str,
        public_key: Key,
        private_key: PrivateKey,
        default_ttl: Duration,
        is_draft: bool,
    ) -> Result<SeriesOwner, crate::Error> {
        let owner = SeriesOwner {
            name: name.to_owned(),
            keypair: Keypair {
                public: public_key.into_inner(),
                secret: private_key.into_inner(),
            },
            default_ttl,
            is_draft,
        };

        let mut batch = WriteBatch::default();

        owner.insert(&mut batch);

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

    pub fn get_all() -> Result<Vec<SeriesOwner>, crate::Error> {
        db().iterator_cf(Table::SeriesOwners.get(), IteratorMode::Start)
            .map(|(_, value)| Ok(bincode::deserialize(&value)?))
            .collect::<Result<Vec<_>, crate::Error>>()
    }

    pub fn series(&self) -> SeriesRef {
        SeriesRef {
            public_key: Key::new(self.keypair.public),
        }
    }

    fn sign(&self, collection: CollectionRef, ttl: Option<Duration>) -> SeriesItem {
        SeriesItem {
            signed: Signed::new(
                SeriesItemContent {
                    collection,
                    timestamp: chrono::Utc::now().trunc_subsecs(0),
                    ttl: ttl.unwrap_or(self.default_ttl),
                },
                &self.keypair,
            ),
            public_key: Key::new(self.keypair.public),
            freshness: chrono::Utc::now(),
            is_draft: self.is_draft,
        }
    }

    pub fn advance(
        &self,
        collection: CollectionRef,
        ttl: Option<Duration>,
    ) -> Result<SeriesItem, crate::Error> {
        let mut batch = WriteBatch::default();

        // But first, unbookmark all your old assets...
        if let Some(item) = db().get_cf(Table::SeriesItems.get(), self.keypair.public)? {
            let item: SeriesItem = bincode::deserialize(&item)?;
            for object in item.collection().list_objects() {
                object?
                    .bookmark(BookmarkType::Reference)
                    .unmark_with(&mut batch);
            }
        }

        // ... and bookmark all your new ones
        for object in collection.list_objects() {
            object?
                .bookmark(BookmarkType::Reference)
                .mark_with(&mut batch);
        }

        let item = self.sign(collection, ttl);

        batch.put_cf(
            Table::SeriesItems.get(),
            item.key(),
            bincode::serialize(&item).expect("can serialize"),
        );

        db().write(batch)?;

        Ok(item)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SeriesRef {
    pub public_key: Key,
}

impl Display for SeriesRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", base64_url::encode(self.key()),)
    }
}

impl Dropable for SeriesRef {
    fn drop_if_exists_with(&self, batch: &mut WriteBatch) -> Result<(), crate::Error> {
        batch.delete_cf(Table::SeriesItems.get(), self.key());
        batch.delete_cf(Table::Series.get(), self.key());

        Ok(())
    }
}

impl SeriesRef {
    pub fn new(public_key: Key) -> SeriesRef {
        SeriesRef { public_key }
    }

    pub fn key(&self) -> &[u8] {
        self.public_key.as_bytes()
    }

    pub fn find(riddle: &ContentRiddle) -> Option<SeriesRef> {
        let it = db().iterator_cf(Table::Series.get(), IteratorMode::Start);

        for (key, value) in it {
            match Key::from_bytes(&key) {
                Ok(key) => {
                    if riddle.resolves(&key.hash()) {
                        match bincode::deserialize(&value) {
                            Ok(series) => return Some(series),
                            Err(err) => {
                                log::warn!("{}", err);
                                break;
                            }
                        }
                    }
                }
                Err(err) => {
                    log::warn!("{}", err);
                    continue;
                }
            }
        }

        None
    }

    pub fn is_locally_owned(&self) -> Result<bool, crate::Error> {
        // TODO: make this not a SeqScan, perhaps?
        for (_, owner) in db().iterator_cf(Table::SeriesOwners.get(), IteratorMode::Start) {
            let owner: SeriesOwner = bincode::deserialize(&owner)?;
            if self.public_key.as_ref() == &owner.keypair.public {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Returns the latest fresh collection in the local database _or_ the latest collection if the
    /// series is locally owned.
    pub fn get_latest_fresh(&self) -> Result<Option<SeriesItem>, crate::Error> {
        let is_locally_owned = self.is_locally_owned()?;

        if let Some(value) = db().get_cf(Table::SeriesItems.get(), self.key())? {
            let item: SeriesItem = bincode::deserialize(&value)?;

            if is_locally_owned || item.is_fresh() {
                Ok(Some(item))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    /// Returns the latest collection in the local database, no matter the freshness or
    /// local ownership.
    pub fn get_latest(&self) -> Result<Option<SeriesItem>, crate::Error> {
        if let Some(value) = db().get_cf(Table::SeriesItems.get(), self.key())? {
            let item: SeriesItem = bincode::deserialize(&value)?;
            Ok(Some(item))
        } else {
            Ok(None)
        }
    }

    pub fn advance(&self, series_item: &SeriesItem) -> Result<(), crate::Error> {
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
    public_key: Key,
    /// Remember to clear this field when sending to the wire.
    freshness: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    is_draft: bool,
}

impl SeriesItem {
    pub fn collection(&self) -> CollectionRef {
        self.signed.collection.clone()
    }

    pub fn is_draft(&self) -> bool {
        self.is_draft
    }

    pub fn is_valid(&self) -> bool {
        self.signed.verify(self.public_key.as_ref())
    }

    pub fn public_key(&self) -> &Key {
        &self.public_key
    }

    #[inline(always)]
    fn key(&self) -> &[u8] {
        // let timestamp = self.signed.timestamp.timestamp_millis();
        // [
        //     self.public_key.as_bytes().as_ref(),
        //     timestamp.to_be_bytes().as_ref(),
        // ]
        // .concat()
        self.public_key.as_bytes()
    }

    pub fn erase_freshness(&mut self) {
        self.freshness =
            chrono::DateTime::from_utc(chrono::NaiveDateTime::from_timestamp(0, 0), chrono::Utc);
    }

    pub fn make_fresh(&mut self) {
        self.freshness = chrono::Utc::now();
    }

    pub fn is_fresh(&self) -> bool {
        chrono::Utc::now()
            < self.freshness
                + chrono::Duration::from_std(self.signed.ttl)
                    .expect("can convert from std duration")
    }

    pub fn freshness(&self) -> chrono::DateTime<chrono::Utc> {
        self.freshness
    }
}

#[test]
fn generate_series_ownership() {
    println!(
        "{}",
        SeriesOwner::create("a series", Duration::from_secs(3600), true)
            .unwrap()
            .series()
    );
}

#[test]
fn validate_series_item() {
    let owner = SeriesOwner::create("a series", Duration::from_secs(3600), true).unwrap();
    let _series = owner.series();

    let current_collection = CollectionRef::rand();

    let series_item = owner.sign(current_collection, None);

    assert!(series_item.is_valid())
}

#[test]
fn not_validate_series_item() {
    let owner = SeriesOwner::create("a series", Duration::from_secs(3600), true).unwrap();

    let other_owner =
        SeriesOwner::create("another series", Duration::from_secs(3600), true).unwrap();
    let other_series = other_owner.series();

    let current_collection = CollectionRef::rand();

    let mut series_item = owner.sign(current_collection, None);
    series_item.public_key = other_series.public_key;

    assert!(!series_item.is_valid())
}
