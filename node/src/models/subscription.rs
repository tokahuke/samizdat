//! A subscription is an active effort from the node to keep the full state of a given
//! series as up-to-date as possible.

use rocksdb::{IteratorMode, WriteBatch};
use serde_derive::{Deserialize, Serialize};
use std::fmt::{self, Display};
use tokio::task::JoinHandle;

use samizdat_common::{Hint, Key, Riddle};

use crate::db::Table;
use crate::{db, hubs};

use super::{Droppable, SeriesRef};

/// The regimen of this subscription. Currently, only downloading the full inventory of
/// the most current edition is supported.
#[derive(Debug, Default, Serialize, Deserialize)]
pub enum SubscriptionKind {
    /// Download the full inventory of the edition, as described in the
    /// [`super::collection::Inventory`].
    #[default]
    FullInventory,
}

/// A subscription is an active effort from the node to keep the full state of a given
/// series as up-to-date as possible.
#[derive(Debug, Serialize, Deserialize)]
pub struct Subscription {
    /// The public key corresponding to the series to be listened to.
    public_key: Key,
    /// The regimen of this subscription.
    kind: SubscriptionKind,
}

impl Subscription {
    /// Creates a new subscription.
    pub fn new(public_key: Key, kind: SubscriptionKind) -> Subscription {
        Subscription { public_key, kind }
    }
}

/// A reference to a subscription.
#[derive(Debug, Serialize, Deserialize)]
pub struct SubscriptionRef {
    /// The public key corresponding to the series to be listened to.
    pub public_key: Key,
}

impl Display for SubscriptionRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "subscription to {}", base64_url::encode(self.key()),)
    }
}

impl Droppable for SubscriptionRef {
    fn drop_if_exists_with(&self, batch: &mut WriteBatch) -> Result<(), crate::Error> {
        batch.delete_cf(Table::Subscriptions.get(), self.key());
        Ok(())
    }
}

impl SubscriptionRef {
    /// Creates a new subscription reference from a given series public key.
    pub fn new(public_key: Key) -> SubscriptionRef {
        SubscriptionRef { public_key }
    }

    // pub fn public_key(&self) -> Key {
    //     self.public_key.clone()
    // }

    /// Whether the subscription exists in the local database.
    pub fn exists(&self) -> Result<bool, crate::Error> {
        Ok(db()
            .get_cf(Table::Subscriptions.get(), self.public_key.as_bytes())?
            .is_some())
    }

    /// The key of this subscription in the database.
    pub fn key(&self) -> &[u8] {
        self.public_key.as_bytes()
    }

    /// Creates a subscription and inserts it into the database.
    pub fn build(subscription: Subscription) -> Result<SubscriptionRef, crate::Error> {
        let mut batch = rocksdb::WriteBatch::default();

        batch.put_cf(
            Table::Subscriptions.get(),
            subscription.public_key.as_bytes(),
            bincode::serialize(&subscription).expect("can serialize"),
        );

        db().write(batch)?;

        let subscription_ref = SubscriptionRef {
            public_key: subscription.public_key,
        };

        subscription_ref.trigger_manual_refresh();

        Ok(subscription_ref)
    }

    /// Triggers a manual refresh (one not initiated by the network) _asynchronously_.
    pub fn trigger_manual_refresh(&self) -> JoinHandle<()> {
        let series = SeriesRef::new(self.public_key.clone());
        tokio::spawn(async move {
            if let Some(latest) = hubs().get_latest(&series).await {
                latest
                    .refresh()
                    .await
                    .map_err(|err| {
                        log::error!("While refreshing {series} with {latest:?}, node got: {err}");
                    })
                    .ok();
            } else {
                log::warn!(
                    "Subscription for {series} was not able to find any edition for this series"
                );
            }
        })
    }

    /// Gets the subscription corresponding to this reference in the database, if it
    /// exists.
    pub fn get(&self) -> Result<Option<Subscription>, crate::Error> {
        let maybe_value = db().get_cf(Table::Subscriptions.get(), self.key())?;
        Ok(maybe_value
            .map(|value| bincode::deserialize(&value))
            .transpose()?)
    }

    /// Gets all subscriptions currently in the database.
    pub fn get_all() -> Result<Vec<Subscription>, crate::Error> {
        db().iterator_cf(Table::Subscriptions.get(), IteratorMode::Start)
            .map(|item| {
                let (_, value) = item?;
                Ok(bincode::deserialize(&value)?)
            })
            .collect::<Result<Vec<_>, crate::Error>>()
    }

    /// Runs through the database looking for a subscription the matches the supplied
    /// riddle. Returns `None` if no subscription matches the riddle.
    pub fn find(riddle: &Riddle, hint: &Hint) -> Result<Option<SubscriptionRef>, crate::Error> {
        let it = db().prefix_iterator_cf(Table::Subscriptions.get(), hint.prefix());

        for item in it {
            let (key, value) = item?;
            match Key::from_bytes(&key) {
                Ok(key) => {
                    if riddle.resolves(&key.hash()) {
                        match bincode::deserialize(&value) {
                            Ok(subscription) => return Ok(Some(subscription)),
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

    /// Reserved for future use.
    pub fn must_refresh(&self) -> Result<bool, crate::Error> {
        Ok(true)
    }
}
