//! Series are temporal sequences of collections that are authenticated by the same
//! private key.

use chrono::Utc;
use ed25519_dalek::SigningKey;
use futures::prelude::*;
use samizdat_common::db::{readonly_tx, writable_tx, Droppable, Table as _, TxHandle, WritableTx};
use samizdat_common::rpc::QueryKind;
use serde_derive::{Deserialize, Serialize};
use std::fmt::{self, Display};
use std::str::FromStr;
use std::time::Duration;
use tokio::time::Instant;

use samizdat_common::cipher::{OpaqueEncrypted, TransferCipher};
use samizdat_common::Hint;
use samizdat_common::HASH_LEN;
use samizdat_common::{rpc::EditionAnnouncement, Hash, Key, PrivateKey, Riddle, Signed};

use crate::db::Table;
use crate::system::ReceivedItem;
use crate::{cli, hubs};

use super::{BookmarkType, CollectionRef, Inventory, ItemPath, ObjectRef};

/// A public-private keypair that allows one to publish new collections
#[derive(Debug, Serialize, Deserialize)]
pub struct SeriesOwner {
    /// An _internal_ name to identify this keypair.
    name: String,
    /// The keypair that controls the series.
    keypair: SigningKey,
    /// The default time-to-leave. This is the recommended minimum period peers should
    /// wait to query the network for new connections.
    #[serde(with = "humantime_serde")]
    default_ttl: Duration,
    /// Whether this series is a draft. Draft series cannot be shared with the Samizdat
    /// network.
    #[serde(default)]
    is_draft: bool,
}

impl Droppable for SeriesOwner {
    fn drop_if_exists_with(&self, tx: &mut WritableTx<'_>) -> Result<(), crate::Error> {
        // Bad idea to drop series and not really worth it space wise.
        // self.series().drop_if_exists_with(batch)?; // bad!

        Table::SeriesOwners.delete(tx, self.name.as_str());
        Ok(())
    }
}

impl SeriesOwner {
    /// Inserts the series owner using the supplied [`WriteBatch`].
    fn insert(&self, tx: &mut WritableTx<'_>) {
        let series = self.series();

        Table::SeriesOwners.put(
            tx,
            &self.name,
            bincode::serialize(&self).expect("can serialize"),
        );
        Table::Series.put(
            tx,
            series.key(),
            bincode::serialize(&series).expect("can serialize"),
        );
    }

    /// Creates a new [`SeriesOwner`] and inserts it into the database.
    pub fn create(
        tx: &mut WritableTx,
        name: &str,
        default_ttl: Duration,
        is_draft: bool,
    ) -> Result<SeriesOwner, crate::Error> {
        let owner = SeriesOwner {
            name: name.to_owned(),
            keypair: SigningKey::generate(&mut rand::rngs::OsRng {}),
            default_ttl,
            is_draft,
        };

        owner.insert(tx);
        Ok(owner)
    }

    /// Creates a [`SeriesOwner`] from existing data and inserts it into the database.
    pub fn import(
        tx: &mut WritableTx,
        name: &str,
        private_key: PrivateKey,
        default_ttl: Duration,
        is_draft: bool,
    ) -> Result<SeriesOwner, crate::Error> {
        let owner = SeriesOwner {
            name: name.to_owned(),
            keypair: private_key.into(),
            default_ttl,
            is_draft,
        };

        owner.insert(tx);
        Ok(owner)
    }

    /// Retrieves a series owner from the database using the internal series name.
    pub fn get<Tx: TxHandle>(tx: &Tx, name: &str) -> Result<Option<SeriesOwner>, crate::Error> {
        Ok(Table::SeriesOwners
            .get(tx, name.as_bytes(), |serialized| {
                bincode::deserialize(serialized)
            })
            .transpose()?)
    }

    /// Gets all series owners in this node.
    pub fn get_all<Tx: TxHandle>(tx: &Tx) -> Result<Vec<SeriesOwner>, crate::Error> {
        Table::SeriesOwners
            .range(..)
            .collect(tx, |_, value| Ok(bincode::deserialize(value)?))
    }

    /// Retrieves the series reference for this series owner.
    pub fn series(&self) -> SeriesRef {
        SeriesRef {
            public_key: Key::new(self.keypair.verifying_key()),
        }
    }

    /// Creates a new edition by signing a collection reference. If the supplied
    /// time-to-leave is `None`, the default TTL will be used.
    fn sign(
        &self,
        collection: CollectionRef,
        timestamp: chrono::DateTime<Utc>,
        ttl: Option<Duration>,
        kind: EditionKind,
    ) -> Edition {
        Edition {
            signed: Signed::new(
                EditionContent {
                    kind,
                    collection,
                    timestamp,
                    ttl: ttl.unwrap_or(self.default_ttl),
                },
                &self.keypair,
            ),
            public_key: Key::new(self.keypair.verifying_key()),
            is_draft: self.is_draft,
        }
    }

    /// Advances the series by creating a new edition and inserting it into the database.
    pub fn advance(
        &self,
        tx: &mut WritableTx,
        collection: CollectionRef,
        timestamp: chrono::DateTime<Utc>,
        ttl: Option<Duration>,
        kind: EditionKind,
    ) -> Result<Edition, crate::Error> {
        // But first, unbookmark all your old assets...
        if let Some(edition) = self.series().get_last_edition(tx) {
            for object in edition.collection().list_objects(tx).collect::<Vec<_>>() {
                object?.bookmark(BookmarkType::Reference).unmark(tx);
            }
        }

        // ... and bookmark all your new ones
        for object in collection.list_objects(tx).collect::<Vec<_>>() {
            object?.bookmark(BookmarkType::Reference).mark(tx);
        }

        let edition = self.sign(collection, timestamp, ttl, kind);

        Table::Editions.put(
            tx,
            edition.key(),
            bincode::serialize(&edition).expect("can serialize"),
        );

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
//         batch.delete_cf(Table::Editions.get(), self.key());
//         batch.delete_cf(Table::Series.get(), self.key());
//
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
    pub fn find<Tx: TxHandle>(
        tx: &Tx,
        riddle: &Riddle,
        hint: &Hint,
    ) -> Result<Option<SeriesRef>, crate::Error> {
        let outcome = Table::Series
            .prefix(hint.prefix())
            .for_each(tx, |key, value| match Key::from_bytes(key) {
                Ok(key) => {
                    if riddle.resolves(&key.hash()) {
                        Some(bincode::deserialize(value).expect("can deserialize"))
                    } else {
                        None
                    }
                }
                Err(err) => {
                    tracing::warn!("{}", err);
                    None
                }
            });

        Ok(outcome)
    }

    /// Whether there is a local "series owner" for this series.
    pub fn is_locally_owned<Tx: TxHandle>(&self, tx: &Tx) -> Result<bool, crate::Error> {
        // TODO: make this not a SeqScan, perhaps?
        let outcome = Table::SeriesOwners.range(..).for_each(tx, |_, owner| {
            let owner: SeriesOwner = bincode::deserialize(owner).expect("can deserialize");
            if self.public_key.as_ref() == &owner.keypair.verifying_key() {
                Some(true)
            } else {
                None
            }
        });

        Ok(outcome.unwrap_or(false))
    }

    /// Set this series as just recently refresh.
    pub fn refresh(&self, tx: &mut WritableTx) -> Result<(), crate::Error> {
        tracing::info!("Setting series {self} as fresh");
        Table::SeriesFreshnesses.put(
            tx,
            self.key(),
            bincode::serialize(&chrono::Utc::now()).expect("can serialize"),
        );

        Ok(())
    }

    /// Set this series as just delayed. By now, this is the same as [`SeriesRef::mark_fresh`].
    pub fn mark_delayed(&self, tx: &mut WritableTx) -> Result<(), crate::Error> {
        tracing::info!("Setting series {self} as delayed");
        Table::SeriesFreshnesses.put(
            tx,
            self.key(),
            bincode::serialize(&chrono::Utc::now()).expect("can serialize"),
        );

        Ok(())
    }

    /// Whether this series is still fresh, according to the latest time-to-leave.
    pub fn is_fresh<Tx: TxHandle>(&self, tx: &Tx) -> Result<bool, crate::Error> {
        let is_fresh = if let Some(latest) = self.get_last_edition(tx) {
            Table::SeriesFreshnesses
                .get(tx, self.key(), |freshness| {
                    let freshness: chrono::DateTime<chrono::Utc> = bincode::deserialize(freshness)?;
                    let ttl = chrono::Duration::from_std(latest.signed.ttl)
                        .expect("can convert duration");

                    Result::<_, crate::Error>::Ok(chrono::Utc::now() < freshness + ttl)
                })
                .transpose()?
                .unwrap_or(false)
        } else {
            false
        };

        Ok(is_fresh)
    }

    /// Returns the latest editions for a series in the local database, no matter the
    /// freshness or local ownership. This iterator is guaranteed to yield items in
    /// reverse chronological order.
    pub fn get_editions<Tx: TxHandle>(
        &self,
        tx: &Tx,
    ) -> impl Send + Sync + Iterator<Item = Edition> {
        let all_editions: Vec<Edition> =
            Table::Editions.prefix(self.key()).collect(tx, |_, value| {
                bincode::deserialize(value).expect("can deserialize")
            });

        all_editions.into_iter().rev()
    }

    /// Gets the last edition for this series in the database.
    pub fn get_last_edition<Tx: TxHandle>(&self, tx: &Tx) -> Option<Edition> {
        self.get_editions(tx).next()
    }

    /// Advances the series with the supplied edition, if the edition is valid.
    pub fn advance(&self, tx: &mut WritableTx, edition: &Edition) -> Result<(), crate::Error> {
        if !edition.is_valid() {
            return Err(crate::Error::InvalidEdition);
        }

        if self.public_key != edition.public_key {
            return Err(crate::Error::DifferentPublicKeys);
        }

        // Insert series if you don't have it yet.
        Table::Series.put(
            tx,
            self.key(),
            bincode::serialize(&self).expect("can serialize"),
        );
        Table::Editions.put(
            tx,
            edition.key(),
            bincode::serialize(&edition).expect("can serialize"),
        );

        // TODO: do some cleanup on the old values.

        Ok(())
    }

    /// Gets all the series references in the database.
    pub fn get_all<Tx: TxHandle>(tx: &Tx) -> Result<Vec<SeriesRef>, crate::Error> {
        Table::Series
            .range(..)
            .collect(tx, |_, value| Ok(bincode::deserialize(value)?))
    }
}

/// The kind of an edition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EditionKind {
    /// Forget everything that came before. All the content will start from scratch.
    Base,
    /// Add to what came before. If an item is not found in the current edition, search for the
    /// content in previous editions (unless _explicitely deleted_).
    Layer,
}

/// The content of an edition. This is the data that is assured by the signature of the
/// edition.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct EditionContent {
    /// The kind of this edition.
    kind: EditionKind,
    /// The collection reference of this edition. This includes the root hash of the
    /// collection.
    collection: CollectionRef,
    /// The timestamp at which this collection was created, allegedly. More recent
    /// editions superseeds less recent editions.
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
    /// The kind of this edition.
    pub fn kind(&self) -> EditionKind {
        self.signed.kind
    }

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
    /// editions superseed less recent editions.
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
        let hint = Hint::new(
            Hash::new(&self.public_key.as_bytes()[..HASH_LEN]),
            cli().hint_size as usize,
        );
        let cipher = TransferCipher::new(&content_hash, &rand);
        let edition = OpaqueEncrypted::new(&self, &cipher);

        EditionAnnouncement {
            rand,
            key_riddle,
            hint,
            edition,
        }
    }

    /// Refresh the underlying series using and *already validated* edition.
    pub async fn refresh(&self) -> Result<(), crate::Error> {
        let exp_backoff = || [0, 10, 30, 70, 150].into_iter().map(Duration::from_secs);

        let collection = self.collection();
        let inventory_location = match self.kind() {
            EditionKind::Base => "_inventory".to_owned(),
            EditionKind::Layer => format!("_changelogs/{}", self.timestamp()),
        };
        let inventory_content_hash = collection
            .locator_for(ItemPath::from(inventory_location.as_str()))
            .hash();

        writable_tx(|tx| {
            let series = self.series();
            series.advance(tx, self)?;
            series.refresh(tx)?;
            Ok(())
        })?;

        if let Some(received_item) = hubs()
            .query_with_retry(
                inventory_content_hash,
                QueryKind::Item,
                Instant::now() + Duration::from_secs(60),
                exp_backoff(),
            )
            .await
        {
            let content = match received_item {
                ReceivedItem::NewObject(received_object) => {
                    received_object
                        .into_content_stream()
                        .collect_content()
                        .await?
                }
                ReceivedItem::ExistingObject(object) => object.content()?.expect("object exists"),
            };

            let inventory: Inventory = serde_json::from_slice(&content).map_err(|err| {
                crate::Error::from(format!(
                    "failed to deserialize inventory for edition {:?}: {}",
                    self, err
                ))
            })?;

            // Make the necessary calls indiscriminately:
            for (item_path, hash) in &inventory {
                if !readonly_tx(|tx| ObjectRef::new(*hash).exists(tx))? {
                    let content_hash = collection.locator_for(item_path.as_path()).hash();
                    tokio::spawn(
                        hubs()
                            .query_with_retry(
                                content_hash,
                                QueryKind::Item,
                                Instant::now() + Duration::from_secs(60),
                                exp_backoff(),
                            )
                            .map(|_| ()),
                    );
                } else {
                    tracing::info!("Object {hash} already exists in the database. Skipping");
                }
            }

            return Ok(());
        }

        Err(crate::Error::from(format!(
            "Inventory not found for edition {:?}. Could not refresh",
            self
        )))
    }

    /// Gets all the editions currently in the database.
    pub fn get_all<Tx: TxHandle>(tx: &Tx) -> Result<Vec<Edition>, crate::Error> {
        Table::Editions
            .range(..)
            .collect(tx, |_, value| Ok(bincode::deserialize(value)?))
    }
}

#[test]
fn generate_series_ownership() {
    println!(
        "{}",
        writable_tx(|tx| SeriesOwner::create(tx, "a series", Duration::from_secs(3600), true))
            .unwrap()
            .series()
    );
}

#[test]
fn validate_edition() {
    let owner =
        writable_tx(|tx| SeriesOwner::create(tx, "a series", Duration::from_secs(3600), true))
            .unwrap();
    let _series = owner.series();
    let current_collection = CollectionRef::rand();
    let edition = owner.sign(current_collection, Utc::now(), None, EditionKind::Base);

    assert!(edition.is_valid())
}

#[test]
fn not_validate_edition() {
    let owner =
        writable_tx(|tx| SeriesOwner::create(tx, "a series", Duration::from_secs(3600), true))
            .unwrap();

    let other_owner = writable_tx(|tx| {
        SeriesOwner::create(tx, "another series", Duration::from_secs(3600), true)
    })
    .unwrap();
    let other_series = other_owner.series();

    let current_collection = CollectionRef::rand();

    let mut edition = owner.sign(current_collection, Utc::now(), None, EditionKind::Base);
    edition.public_key = other_series.public_key;

    assert!(!edition.is_valid())
}
