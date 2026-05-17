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

/// Maximum amount by which a freshly-signed edition's `timestamp` may exceed
/// the receiver's wall clock before being rejected. Generous enough to absorb
/// realistic clock skew, tight enough that a compromised owner cannot pin
/// subscribers to a future-dated edition for years.
const EDITION_CLOCK_SKEW: chrono::Duration = chrono::Duration::minutes(5);

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

        Table::SeriesOwners.delete(tx, self.name.as_str())?;
        Ok(())
    }
}

impl SeriesOwner {
    /// Inserts the series owner using the supplied [`WriteBatch`].
    fn insert(&self, tx: &mut WritableTx<'_>) -> Result<(), crate::Error> {
        let series = self.series();

        Table::SeriesOwners.put(
            tx,
            &self.name,
            bincode::serialize(&self).expect("can serialize"),
        )?;
        Table::Series.put(
            tx,
            series.key(),
            bincode::serialize(&series).expect("can serialize"),
        )?;
        Ok(())
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

        owner.insert(tx)?;
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

        owner.insert(tx)?;
        Ok(owner)
    }

    /// Retrieves a series owner from the database using the internal series name.
    pub fn get<Tx: TxHandle>(tx: &Tx, name: &str) -> Result<Option<SeriesOwner>, crate::Error> {
        Table::SeriesOwners.get(tx, name.as_bytes(), |serialized| {
            Ok(bincode::deserialize(serialized)?)
        })
    }

    /// Gets all series owners in this node.
    pub fn get_all<Tx: TxHandle>(tx: &Tx) -> Result<Vec<SeriesOwner>, crate::Error> {
        let collected: Result<Vec<SeriesOwner>, crate::Error> = Table::SeriesOwners
            .range::<_, [u8; 0]>(..)
            .collect(tx, |_, value| {
                Ok::<SeriesOwner, crate::Error>(bincode::deserialize(value)?)
            })?;
        collected
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
        if let Some(edition) = self.series().get_last_edition(tx)? {
            let old_objects: Vec<_> = edition.collection().list_objects(tx)?.collect();
            for object in old_objects {
                object?.bookmark(BookmarkType::Reference).unmark(tx)?;
            }
        }

        // ... and bookmark all your new ones
        let new_objects: Vec<_> = collection.list_objects(tx)?.collect();
        for object in new_objects {
            object?.bookmark(BookmarkType::Reference).mark(tx)?;
        }

        let edition = self.sign(collection, timestamp, ttl, kind);

        Table::Editions.put(
            tx,
            edition.key(),
            bincode::serialize(&edition).expect("can serialize"),
        )?;

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
        let outcome = Table::Series.prefix(hint.prefix()).for_each(tx, |key, value| {
            match Key::from_bytes(key) {
                Ok(key) => {
                    if riddle.resolves(&key.hash()) {
                        Ok(Some(bincode::deserialize(value)?))
                    } else {
                        Ok(None)
                    }
                }
                Err(err) => {
                    tracing::warn!("{}", err);
                    Ok(None)
                }
            }
        })?;

        Ok(outcome)
    }

    /// Whether there is a local "series owner" for this series.
    pub fn is_locally_owned<Tx: TxHandle>(&self, tx: &Tx) -> Result<bool, crate::Error> {
        // TODO: make this not a SeqScan, perhaps?
        let outcome = Table::SeriesOwners
            .range::<_, [u8; 0]>(..)
            .for_each(tx, |_, owner| {
                let owner: SeriesOwner = bincode::deserialize(owner)?;
                if self.public_key.as_ref() == &owner.keypair.verifying_key() {
                    Ok(Some(true))
                } else {
                    Ok(None)
                }
            })?;

        Ok(outcome.unwrap_or(false))
    }

    /// Set this series as just recently refresh.
    pub fn refresh(&self, tx: &mut WritableTx) -> Result<(), crate::Error> {
        tracing::info!("Setting series {self} as fresh");
        Table::SeriesFreshnesses.put(
            tx,
            self.key(),
            bincode::serialize(&chrono::Utc::now()).expect("can serialize"),
        )?;

        Ok(())
    }

    /// Set this series as just delayed. By now, this is the same as [`SeriesRef::mark_fresh`].
    pub fn mark_delayed(&self, tx: &mut WritableTx) -> Result<(), crate::Error> {
        tracing::info!("Setting series {self} as delayed");
        Table::SeriesFreshnesses.put(
            tx,
            self.key(),
            bincode::serialize(&chrono::Utc::now()).expect("can serialize"),
        )?;

        Ok(())
    }

    /// Whether this series is still fresh, according to the latest time-to-leave.
    pub fn is_fresh<Tx: TxHandle>(&self, tx: &Tx) -> Result<bool, crate::Error> {
        let is_fresh = if let Some(latest) = self.get_last_edition(tx)? {
            Table::SeriesFreshnesses
                .get(tx, self.key(), |freshness| {
                    let freshness: chrono::DateTime<chrono::Utc> = bincode::deserialize(freshness)?;
                    // Saturate instead of panicking. A malicious series owner could
                    // sign an edition whose `ttl` exceeds `chrono::Duration::MAX`,
                    // and the old `.expect("can convert duration")` would crash the
                    // read path on every `is_fresh` call.
                    let ttl = chrono::Duration::from_std(latest.signed.ttl)
                        .unwrap_or(chrono::Duration::max_value());

                    Result::<_, crate::Error>::Ok(chrono::Utc::now() < freshness + ttl)
                })?
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
    ) -> Result<impl Send + Sync + Iterator<Item = Edition>, crate::Error> {
        let all_editions: Vec<Edition> =
            Table::Editions.prefix(self.key()).collect(tx, |_, value| {
                bincode::deserialize(value).expect("can deserialize")
            })?;

        Ok(all_editions.into_iter().rev())
    }

    /// Gets the last edition for this series in the database.
    pub fn get_last_edition<Tx: TxHandle>(
        &self,
        tx: &Tx,
    ) -> Result<Option<Edition>, crate::Error> {
        Ok(self.get_editions(tx)?.next())
    }

    /// Advances the series with the supplied edition, if the edition is valid.
    ///
    /// Used by all peer-driven paths (announcement, get_edition, refresh). Performs:
    ///   1. Signature validity check (already done before this call, but defensive).
    ///   2. Key-binding check: edition's public_key must match this SeriesRef.
    ///   3. **Monotonicity check (B5):** rejects editions older than the current latest.
    ///      Defeats rollback replays from a misbehaving peer/hub that re-broadcasts an
    ///      old (still-validly-signed) edition.
    ///   4. **Bookmark accounting (B4):** Reference-pins the new edition's objects and
    ///      Reference-unpins the previous edition's objects, so the vacuum doesn't
    ///      collect inventory that an active subscription wants. Idempotent: advancing
    ///      to the same edition twice (same key) is a no-op.
    pub fn advance(&self, tx: &mut WritableTx, edition: &Edition) -> Result<(), crate::Error> {
        if !edition.is_valid() {
            return Err(crate::Error::InvalidEdition);
        }

        if self.public_key != edition.public_key {
            return Err(crate::Error::DifferentPublicKeys);
        }

        // Refuse editions whose timestamp is too far in the future. Without
        // this bound a compromised series owner could publish an edition with
        // `timestamp = now + 100y` and lock subscribers to it indefinitely:
        // the monotonicity check below would then reject every legitimate
        // republish as stale until real-world clocks catch up. The
        // `EDITION_CLOCK_SKEW` constant lives at module scope.
        let now = chrono::Utc::now();
        if edition.timestamp() > now + EDITION_CLOCK_SKEW {
            return Err(format!(
                "edition timestamp {} is too far in the future (now is {now})",
                edition.timestamp()
            )
            .into());
        }

        let previous_latest = self.get_last_edition(tx)?;

        if let Some(ref latest) = previous_latest {
            // Idempotent re-advance: same edition key; already applied, nothing to do.
            if latest.key() == edition.key() {
                return Ok(());
            }
            // Refuse stale editions.
            if edition.timestamp() < latest.timestamp() {
                return Err(crate::Error::StaleEdition {
                    candidate: edition.timestamp(),
                    current: latest.timestamp(),
                });
            }
        }

        // Reference-unpin the previous edition's objects, if any.
        if let Some(ref latest) = previous_latest {
            let old_objects: Vec<_> = latest.collection().list_objects(tx)?.collect();
            for object in old_objects {
                object?.bookmark(BookmarkType::Reference).unmark(tx)?;
            }
        }

        // Reference-pin the new edition's objects.
        let new_objects: Vec<_> = edition.collection().list_objects(tx)?.collect();
        for object in new_objects {
            object?.bookmark(BookmarkType::Reference).mark(tx)?;
        }

        // Insert series if you don't have it yet.
        Table::Series.put(
            tx,
            self.key(),
            bincode::serialize(&self).expect("can serialize"),
        )?;
        Table::Editions.put(
            tx,
            edition.key(),
            bincode::serialize(&edition).expect("can serialize"),
        )?;

        Ok(())
    }

    /// Gets all the series references in the database.
    pub fn get_all<Tx: TxHandle>(tx: &Tx) -> Result<Vec<SeriesRef>, crate::Error> {
        let collected: Result<Vec<SeriesRef>, crate::Error> = Table::Series
            .range::<_, [u8; 0]>(..)
            .collect(tx, |_, value| {
                Ok::<SeriesRef, crate::Error>(bincode::deserialize(value)?)
            })?;
        collected
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
    ///
    /// Encodes the timestamp at microsecond precision (i64, big-endian). At second
    /// precision two editions cut in the same second would silently overwrite each other
    /// in `Table::Editions`. Microseconds give more than enough headroom (i64 µs
    /// represents ~292 000 years) while keeping the key small.
    #[inline(always)]
    fn key(&self) -> Vec<u8> {
        [
            self.public_key.as_bytes(),
            &self.timestamp().timestamp_micros().to_be_bytes(),
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

    /// Refresh the underlying series using an *already validated* edition.
    ///
    /// Ordering matters here: we fetch and parse the inventory FIRST, only then commit
    /// the `advance` + `refresh` to disk. Previously the advance/refresh happened
    /// upfront; so if the inventory fetch failed, the series was already marked fresh
    /// against a missing edition and `is_fresh` suppressed re-queries for one TTL period.
    /// A hub announcing a valid edition while withholding the inventory could DoS the
    /// subscription for the TTL window. Fetching first means a failed fetch leaves the
    /// previous state intact and the next resolve attempt will re-query naturally.
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

        // 1. Fetch + parse inventory BEFORE persisting any state.
        let Some(received_item) = hubs()
            .query_with_retry(
                inventory_content_hash,
                QueryKind::Item,
                Instant::now() + Duration::from_secs(60),
                exp_backoff(),
            )
            .await
        else {
            return Err(crate::Error::from(format!(
                "Inventory not found for edition {:?}. Could not refresh",
                self
            )));
        };

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

        // 2. Inventory in hand; commit advance + refresh atomically.
        writable_tx(|tx| {
            let series = self.series();
            series.advance(tx, self)?;
            series.refresh(tx)?;
            Ok(())
        })?;

        // 3. Schedule fetches for objects not yet local. (Best-effort; failures of
        //    individual object fetches do not roll back the advance/refresh; the series
        //    is at the new edition, missing objects will be queried on demand.)
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

        Ok(())
    }

    /// Gets all the editions currently in the database.
    pub fn get_all<Tx: TxHandle>(tx: &Tx) -> Result<Vec<Edition>, crate::Error> {
        let collected: Result<Vec<Edition>, crate::Error> = Table::Editions
            .range::<_, [u8; 0]>(..)
            .collect(tx, |_, value| {
                Ok::<Edition, crate::Error>(bincode::deserialize(value)?)
            })?;
        collected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use samizdat_common::db::test_harness::TestDb;
    use samizdat_common::Hash;

    /// Builds a series-owner name unique enough not to collide with leftover state from
    /// previous tests in the same binary (TestDb shares the global DB across tests).
    fn unique_name(base: &str) -> String {
        format!("{base}-{}", Hash::rand())
    }

    #[test]
    fn generate_series_ownership() {
        TestDb::<crate::db::Table>::with(|| {
            let name = unique_name("a-series");
            let series = writable_tx(|tx| {
                SeriesOwner::create(tx, &name, Duration::from_secs(3600), true)
            })
            .unwrap()
            .series();
            // Just check we got a series back; we don't compare structure.
            let _ = series;
        });
    }

    #[test]
    fn validate_edition() {
        TestDb::<crate::db::Table>::with(|| {
            let owner = writable_tx(|tx| {
                SeriesOwner::create(tx, &unique_name("v"), Duration::from_secs(3600), true)
            })
            .unwrap();
            let edition = owner.sign(CollectionRef::rand(), Utc::now(), None, EditionKind::Base);
            assert!(edition.is_valid());
        });
    }

    #[test]
    fn not_validate_edition_with_swapped_key() {
        TestDb::<crate::db::Table>::with(|| {
            let owner = writable_tx(|tx| {
                SeriesOwner::create(tx, &unique_name("nv-a"), Duration::from_secs(3600), true)
            })
            .unwrap();
            let other = writable_tx(|tx| {
                SeriesOwner::create(tx, &unique_name("nv-b"), Duration::from_secs(3600), true)
            })
            .unwrap();

            let mut edition =
                owner.sign(CollectionRef::rand(), Utc::now(), None, EditionKind::Base);
            edition.public_key = other.series().public_key;

            assert!(!edition.is_valid());
        });
    }

    /// Regression test for B5; the key used to be at second granularity, so two
    /// editions cut in the same second overwrote each other. Now we encode microseconds
    /// (12 bytes total: public_key || i64 µs BE).
    #[test]
    fn edition_key_includes_microseconds() {
        TestDb::<crate::db::Table>::with(|| {
            let owner = writable_tx(|tx| {
                SeriesOwner::create(tx, &unique_name("k"), Duration::from_secs(3600), true)
            })
            .unwrap();

            let t = Utc::now();
            let e1 = owner.sign(CollectionRef::rand(), t, None, EditionKind::Base);
            let e2 = owner.sign(
                CollectionRef::rand(),
                t + chrono::Duration::microseconds(1),
                None,
                EditionKind::Base,
            );

            assert_ne!(e1.key(), e2.key(), "1µs-apart editions must produce distinct keys");
        });
    }

    /// Regression test for B5; `SeriesRef::advance` must reject editions older than
    /// the current latest to defeat rollback replays from a misbehaving peer.
    #[test]
    fn advance_rejects_stale_edition() {
        TestDb::<crate::db::Table>::with(|| {
            let owner = writable_tx(|tx| {
                SeriesOwner::create(tx, &unique_name("stale"), Duration::from_secs(3600), true)
            })
            .unwrap();
            let series = owner.series();

            let t_new = Utc::now();
            let newer = owner.sign(CollectionRef::rand(), t_new, None, EditionKind::Base);
            let older = owner.sign(
                CollectionRef::rand(),
                t_new - chrono::Duration::seconds(60),
                None,
                EditionKind::Base,
            );

            writable_tx(|tx| series.advance(tx, &newer)).unwrap();

            let result = writable_tx(|tx| series.advance(tx, &older));
            assert!(
                matches!(result, Err(crate::Error::StaleEdition { .. })),
                "stale edition was accepted: {result:?}"
            );
        });
    }

    /// Regression test for B5; re-advancing to the SAME edition (same key) must be an
    /// idempotent no-op (not a stale rejection, not a duplicate insert).
    #[test]
    fn advance_is_idempotent_for_same_edition() {
        TestDb::<crate::db::Table>::with(|| {
            let owner = writable_tx(|tx| {
                SeriesOwner::create(tx, &unique_name("idem"), Duration::from_secs(3600), true)
            })
            .unwrap();
            let series = owner.series();
            let edition = owner.sign(CollectionRef::rand(), Utc::now(), None, EditionKind::Base);

            writable_tx(|tx| series.advance(tx, &edition)).unwrap();
            // Second call: must not error.
            writable_tx(|tx| series.advance(tx, &edition)).unwrap();
        });
    }

    /// Regression test for the cross-key forgery; `SeriesRef::advance` already
    /// rejected via `DifferentPublicKeys`; pin that behavior.
    #[test]
    fn advance_rejects_wrong_public_key() {
        TestDb::<crate::db::Table>::with(|| {
            let owner = writable_tx(|tx| {
                SeriesOwner::create(tx, &unique_name("wk-a"), Duration::from_secs(3600), true)
            })
            .unwrap();
            let other = writable_tx(|tx| {
                SeriesOwner::create(tx, &unique_name("wk-b"), Duration::from_secs(3600), true)
            })
            .unwrap();

            let edition = owner.sign(CollectionRef::rand(), Utc::now(), None, EditionKind::Base);
            // The series we ask to advance with this edition is the OTHER one.
            let result = writable_tx(|tx| other.series().advance(tx, &edition));
            assert!(matches!(
                result,
                Err(crate::Error::DifferentPublicKeys)
            ));
        });
    }
}
