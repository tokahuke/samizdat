//! Series are temporal sequences of collections that are authenticated by the same
//! private key.

use chrono::SubsecRound;
use ed25519_dalek::Keypair;
use rocksdb::{IteratorMode, WriteBatch};
use serde_derive::{Deserialize, Serialize};
use std::fmt::{self, Display};
use std::str::FromStr;
use std::time::Duration;

use samizdat_common::cipher::{OpaqueEncrypted, TransferCipher};
use samizdat_common::{rpc::EditionAnnouncement, Hash, Key, PrivateKey, Riddle, Signed};

use crate::db;
use crate::db::Table;

use super::{BookmarkType, CollectionRef, Droppable};

/// A public-private keypair that allows one to publish new collections
#[derive(Debug, Serialize, Deserialize)]
pub struct SeriesOwner {
    /// An _internal_ name to identify this keypair.
    name: String,
    /// The keypair that controls the series.
    keypair: Keypair,
    /// The default time-to-leave. This is the reccommended minimum period peers should
    /// wait to query the network for new connections.
    #[serde(with = "humantime_serde")]
    default_ttl: Duration,
    /// Whether this series is a draft. Draft series cannot be shared with the Samizdat
    /// network.
    #[serde(default)]
    is_draft: bool,
}

impl Droppable for SeriesOwner {
    fn drop_if_exists_with(&self, batch: &mut WriteBatch) -> Result<(), crate::Error> {
        // Bad idea to drop series and not really worth it space wise.
        // self.series().drop_if_exists_with(batch)?;
        batch.delete_cf(Table::SeriesOwners.get(), self.name.as_bytes());

        Ok(())
    }
}

impl SeriesOwner {
    /// Inserts the series owner using the supplied [`WriteBatch`].
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

    /// Creates a new [`SeriesOwner`] and inserts it into the database.
    pub fn create(
        name: &str,
        default_ttl: Duration,
        is_draft: bool,
    ) -> Result<SeriesOwner, crate::Error> {
        let owner = SeriesOwner {
            name: name.to_owned(),
            keypair: Keypair::generate(&mut rand::rngs::OsRng {}),
            default_ttl,
            is_draft,
        };

        let mut batch = WriteBatch::default();

        owner.insert(&mut batch);

        db().write(batch)?;

        Ok(owner)
    }

    /// Creates a [`SeriesOwner`] from existing data and inserts it into the database.
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

    /// Retrieves a series owner from the database using the internal series name.
    pub fn get(name: &str) -> Result<Option<SeriesOwner>, crate::Error> {
        let maybe_serialized = db().get_cf(Table::SeriesOwners.get(), name.as_bytes())?;
        if let Some(serialized) = maybe_serialized {
            let owner = bincode::deserialize(&serialized)?;
            Ok(Some(owner))
        } else {
            Ok(None)
        }
    }

    /// Gets all series owners in this node.
    pub fn get_all() -> Result<Vec<SeriesOwner>, crate::Error> {
        db().iterator_cf(Table::SeriesOwners.get(), IteratorMode::Start)
            .map(|item| {
                let (_, value) = item?;
                Ok(bincode::deserialize(&value)?)
            })
            .collect::<Result<Vec<_>, crate::Error>>()
    }

    /// Retrieves the series reference for this series owner.
    pub fn series(&self) -> SeriesRef {
        SeriesRef {
            public_key: Key::new(self.keypair.public),
        }
    }

    /// Creates a new edition by signing a collection reference. If the supplied
    /// time-to-leave is `None`, the default TTL will be used.
    fn sign(&self, collection: CollectionRef, ttl: Option<Duration>) -> Edition {
        Edition {
            signed: Signed::new(
                EditionContent {
                    collection,
                    timestamp: chrono::Utc::now().trunc_subsecs(0),
                    ttl: ttl.unwrap_or(self.default_ttl),
                },
                &self.keypair,
            ),
            public_key: Key::new(self.keypair.public),
            is_draft: self.is_draft,
        }
    }

    /// Advances the series by creating a new edition and inserting it into the database.
    pub fn advance(
        &self,
        collection: CollectionRef,
        ttl: Option<Duration>,
    ) -> Result<Edition, crate::Error> {
        let mut batch = WriteBatch::default();

        // But first, unbookmark all your old assets...
        if let Some(edition) = self.series().get_editions()?.first() {
            for object in edition.collection().list_objects() {
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

        let edition = self.sign(collection, ttl);

        batch.put_cf(
            Table::Editions.get(),
            edition.key(),
            bincode::serialize(&edition).expect("can serialize"),
        );

        db().write(batch)?;

        Ok(edition)
    }
}

/// A reference to a series.
#[derive(Debug, Serialize, Deserialize)]
pub struct SeriesRef {
    /// The public key that defines this series.
    pub public_key: Key,
}

impl Display for SeriesRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", base64_url::encode(self.key()),)
    }
}

impl FromStr for SeriesRef {
    type Err = crate::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(SeriesRef {
            public_key: s.parse()?,
        })
    }
}

// Bad idea to drop series and not really worth it space wise.

// impl Droppable for SeriesRef {
//     fn drop_if_exists_with(&self, batch: &mut WriteBatch) -> Result<(), crate::Error> {
//         // TODO: no, you dummy! Drop _prefix_
//         // TODO: and don't forget to drop
//         batch.delete_cf(Table::Editions.get(), self.key());
//         batch.delete_cf(Table::Series.get(), self.key());

//         Ok(())
//     }
// }

impl SeriesRef {
    /// Creates a new series reference from a given public key.
    pub fn new(public_key: Key) -> SeriesRef {
        SeriesRef { public_key }
    }

    /// The public key that defines this series.
    pub fn public_key(&self) -> Key {
        self.public_key.clone()
    }

    /// The public key that defines this series, as binary data.
    pub fn key(&self) -> &[u8] {
        self.public_key.as_bytes()
    }

    /// Runs through the database looking for a series that matches the supplied riddle.
    /// Returns `Ok(None)` if none is found.
    pub fn find(riddle: &Riddle) -> Result<Option<SeriesRef>, crate::Error> {
        let it = db().iterator_cf(Table::Series.get(), IteratorMode::Start);

        for item in it {
            let (key, value) = item?;
            match Key::from_bytes(&key) {
                Ok(key) => {
                    if riddle.resolves(&key.hash()) {
                        match bincode::deserialize(&value) {
                            Ok(series) => return Ok(Some(series)),
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

        Ok(None)
    }

    /// Whether there is a local "series owner" for this series.
    pub fn is_locally_owned(&self) -> Result<bool, crate::Error> {
        // TODO: make this not a SeqScan, perhaps?
        for item in db().iterator_cf(Table::SeriesOwners.get(), IteratorMode::Start) {
            let (_, owner) = item?;
            let owner: SeriesOwner = bincode::deserialize(&owner)?;
            if self.public_key.as_ref() == &owner.keypair.public {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Set this series as just recently refresh.
    pub fn refresh(&self) -> Result<(), crate::Error> {
        log::info!("Setting series {self} as fresh");
        db().put_cf(
            Table::SeriesFreshnesses.get(),
            self.key(),
            bincode::serialize(&chrono::Utc::now()).expect("can serialize"),
        )?;

        Ok(())
    }

    /// Set this series as just delayed.
    /// TODO: should get a better implementation in the future. By now, same as refresh.
    pub fn mark_delayed(&self) -> Result<(), crate::Error> {
        log::info!("Setting series {self} as fresh");
        db().put_cf(
            Table::SeriesFreshnesses.get(),
            self.key(),
            bincode::serialize(&chrono::Utc::now()).expect("can serialize"),
        )?;

        Ok(())
    }

    /// Whether this series is still fresh, according to the latest time-to-leave.
    pub fn is_fresh(&self) -> Result<bool, crate::Error> {
        let is_fresh = if let Some(latest) = self.get_editions()?.first() {
            if let Some(freshness) = db().get_cf(Table::SeriesFreshnesses.get(), self.key())? {
                let freshness: chrono::DateTime<chrono::Utc> = bincode::deserialize(&freshness)?;
                let ttl =
                    chrono::Duration::from_std(latest.signed.ttl).expect("can convert duration");

                chrono::Utc::now() < freshness + ttl
            } else {
                false
            }
        } else {
            false
        };

        Ok(is_fresh)
    }

    /// Returns the latest collection in the local database, no matter the freshness or
    /// local ownership.
    ///
    /// TODO: should return iterator, since normally only the latest editions are important.
    /// Although I regard this impl as safer, in a first moment.
    pub fn get_editions(&self) -> Result<Vec<Edition>, crate::Error> {
        let prefix = self.key();
        let mut editions = db()
            .prefix_iterator_cf(Table::Editions.get(), prefix)
            .map(|item| {
                let (_key, value) = item?;
                let edition: Edition = bincode::deserialize(&value)?;
                Ok(edition)
            })
            .collect::<Result<Vec<_>, crate::Error>>()?;

        // Probably already sorted, but...
        editions.sort_unstable_by_key(|edition| std::cmp::Reverse(edition.timestamp()));

        Ok(editions)
    }

    /// Advances the series with the supplied edition, if the edition is valid.
    pub fn advance(&self, edition: &Edition) -> Result<(), crate::Error> {
        if !edition.is_valid() {
            return Err(crate::Error::InvalidEdition);
        }

        if self.public_key != edition.public_key {
            return Err(crate::Error::DifferentPublicKeys);
        }

        let mut batch = rocksdb::WriteBatch::default();

        // Insert series if you don't have it yet.
        batch.put_cf(
            Table::Series.get(),
            self.key(),
            bincode::serialize(&self).expect("can serialize"),
        );
        batch.put_cf(
            Table::Editions.get(),
            edition.key(),
            bincode::serialize(&edition).expect("can serialize"),
        );

        db().write(batch)?;

        // TODO: do some cleanup on the old values.

        Ok(())
    }

    /// Gets all the series references in the database.
    pub fn get_all() -> Result<Vec<SeriesRef>, crate::Error> {
        db().iterator_cf(Table::Series.get(), IteratorMode::Start)
            .map(|item| {
                let (_, value) = item?;
                Ok(bincode::deserialize(&value)?)
            })
            .collect::<Result<Vec<_>, crate::Error>>()
    }
}

/// The content of an edition. This is the data that is assured by the signature of the
/// edition.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct EditionContent {
    /// The collection reference of this edition. This includes the root hash of the
    /// collection.
    collection: CollectionRef,
    /// The timestamp at which this collection was created, allegedly. More recent
    /// editions supeseed less recent editions.
    timestamp: chrono::DateTime<chrono::Utc>,
    /// The recommended time-to-leave for this edition.
    #[serde(with = "humantime_serde")]
    ttl: Duration,
}

/// An edition of a series.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edition {
    /// The signed content of this edition.
    signed: Signed<EditionContent>,
    /// The public key that signed this edition.
    public_key: Key,
    /// Whether this edition is a draft. Draft editions are not shared with the
    /// Samizdat network.
    #[serde(default)]
    is_draft: bool,
}

impl Edition {
    /// The collection pointed by this edition.
    pub fn collection(&self) -> CollectionRef {
        self.signed.collection.clone()
    }

    /// Whether this edition is a draft. Draft editions are not shared with the
    /// Samizdat network.
    pub fn is_draft(&self) -> bool {
        self.is_draft
    }

    /// Whether the signature is verified by the supplied key in the edition.
    pub fn is_valid(&self) -> bool {
        self.signed.verify(self.public_key.as_ref())
    }

    /// The public key associated with this edition.
    pub fn public_key(&self) -> &Key {
        &self.public_key
    }

    /// The series reference associated with this edition.
    pub fn series(&self) -> SeriesRef {
        SeriesRef {
            public_key: self.public_key.clone(),
        }
    }

    /// The key of this edition in the database.
    #[inline(always)]
    fn key(&self) -> Vec<u8> {
        [
            self.public_key.as_bytes(),
            &self.timestamp().timestamp().to_be_bytes(),
        ]
        .concat()
    }

    /// The timestamp at which this collection was created, allegedly. More recent
    /// editions supeseed less recent editions.
    #[inline(always)]
    pub fn timestamp(&self) -> chrono::DateTime<chrono::Utc> {
        self.signed.timestamp
    }

    /// Creates an announcement for this edition. Announcements can be shared with the
    /// Samizdat network without revealing the public key associated with it.
    pub fn announcement(&self) -> EditionAnnouncement {
        let rand = Hash::rand();
        let content_hash = self.public_key.hash();
        let key_riddle = Riddle::new(&content_hash);
        let cipher = TransferCipher::new(&content_hash, &rand);
        let edition = OpaqueEncrypted::new(&self, &cipher);

        EditionAnnouncement {
            rand,
            key_riddle,
            edition,
        }
    }

    /// Gets all the editions currently in the database.
    pub fn get_all() -> Result<Vec<Edition>, crate::Error> {
        db().iterator_cf(Table::Editions.get(), IteratorMode::Start)
            .map(|item| {
                let (_, value) = item?;
                Ok(bincode::deserialize(&value)?)
            })
            .collect::<Result<Vec<_>, crate::Error>>()
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
fn validate_edition() {
    let owner = SeriesOwner::create("a series", Duration::from_secs(3600), true).unwrap();
    let _series = owner.series();
    let current_collection = CollectionRef::rand();
    let edition = owner.sign(current_collection, None);

    assert!(edition.is_valid())
}

#[test]
fn not_validate_edition() {
    let owner = SeriesOwner::create("a series", Duration::from_secs(3600), true).unwrap();

    let other_owner =
        SeriesOwner::create("another series", Duration::from_secs(3600), true).unwrap();
    let other_series = other_owner.series();

    let current_collection = CollectionRef::rand();

    let mut edition = owner.sign(current_collection, None);
    edition.public_key = other_series.public_key;

    assert!(!edition.is_valid())
}
