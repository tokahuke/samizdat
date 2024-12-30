//! A subscription is an active effort from the node to keep the full state of a given
//! series as up-to-date as possible.

use jammdb::Tx;
use serde_derive::{Deserialize, Serialize};
use std::fmt::{self, Display};
use tokio::task::JoinHandle;

use samizdat_common::{Hint, Key, Riddle};

use crate::db::Table;
use crate::hubs;

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
    fn drop_if_exists_with(&self, tx: &Tx<'_>) -> Result<(), crate::Error> {
        Table::Subscriptions.delete(tx, self.key());
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
        Ok(Table::Subscriptions.atomic_has(self.public_key.as_bytes()))
    }

    /// The key of this subscription in the database.
    pub fn key(&self) -> &[u8] {
        self.public_key.as_bytes()
    }

    /// Creates a subscription and inserts it into the database.
    pub fn build(subscription: Subscription) -> Result<SubscriptionRef, crate::Error> {
        Table::Subscriptions.atomic_put(
            subscription.public_key.as_bytes(),
            bincode::serialize(&subscription).expect("can serialize"),
        );

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
                        tracing::error!(
                            "While refreshing {series} with {latest:?}, node got: {err}"
                        );
                    })
                    .ok();
            } else {
                tracing::warn!(
                    "Subscription for {series} was not able to find any edition for this series"
                );
            }
        })
    }

    /// Gets the subscription corresponding to this reference in the database, if it
    /// exists.
    pub fn get(&self) -> Result<Option<Subscription>, crate::Error> {
        Ok(Table::Subscriptions
            .atomic_get(self.key(), |value| bincode::deserialize(&value))
            .transpose()?)
    }

    /// Gets all subscriptions currently in the database.
    pub fn get_all() -> Result<Vec<Subscription>, crate::Error> {
        Table::Subscriptions
            .range(..)
            .atomic_collect(|_, value| Ok(bincode::deserialize(&value)?))
    }

    /// Runs through the database looking for a subscription the matches the supplied
    /// riddle. Returns `None` if no subscription matches the riddle.
    pub fn find(riddle: &Riddle, hint: &Hint) -> Result<Option<SubscriptionRef>, crate::Error> {
        let outcome = Table::Subscriptions
            .prefix(hint.prefix())
            .atomic_for_each(|key, value| match Key::from_bytes(&key) {
                Ok(key) => {
                    if riddle.resolves(&key.hash()) {
                        Some(bincode::deserialize(&value).expect("can deserialize"))
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

    /// Reserved for future use.
    pub fn must_refresh(&self) -> Result<bool, crate::Error> {
        Ok(true)
    }
}
