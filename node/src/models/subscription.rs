//! A subscription is an active effort from the node to keep the full state of a given
//! series as up-to-date as possible.

use futures::prelude::*;
use rocksdb::{IteratorMode, WriteBatch};
use serde_derive::{Deserialize, Serialize};
use std::fmt::{self, Display};

use samizdat_common::rpc::QueryKind;
use samizdat_common::{Key, Riddle};

use crate::db;
use crate::db::Table;
use crate::hubs;

use super::{Droppable, Edition, Inventory};

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

    /// The key of this subscription in the database.
    pub fn key(&self) -> &[u8] {
        self.public_key.as_bytes()
    }

    /// Creates a subscription and inserts it into the database.
    pub fn build(subscription: Subscription) -> Result<SubscriptionRef, crate::Error> {
        let mut batch = rocksdb::WriteBatch::default();

        batch.put_cf(
            Table::Subscriptions.get(),
            &subscription.public_key.as_bytes(),
            bincode::serialize(&subscription).expect("can serialize"),
        );

        db().write(batch)?;

        Ok(SubscriptionRef {
            public_key: subscription.public_key,
        })
    }

    /// Gets the subscription corresponding to this reference in the database, if it
    /// exists.
    pub fn get(&self) -> Result<Option<Subscription>, crate::Error> {
        let maybe_value = db().get_cf(Table::Subscriptions.get(), &self.key())?;
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
    pub fn find(riddle: &Riddle) -> Result<Option<SubscriptionRef>, crate::Error> {
        let it = db().iterator_cf(Table::Subscriptions.get(), IteratorMode::Start);

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

    /// Refresh the underlying series using and *already validated* edition.
    pub async fn refresh(&self, edition: Edition) -> Result<(), crate::Error> {
        let collection = edition.collection();
        let inventory_content_hash = collection.locator_for("_inventory".into()).hash();

        let series = edition.series();
        series.advance(&edition)?;
        series.refresh()?;

        if let Some(received_object) = hubs().query(inventory_content_hash, QueryKind::Item).await {
            let content = received_object
                .into_content_stream()
                .collect_content()
                .await?;

            let inventory: Inventory = serde_json::from_slice(&content).map_err(|err| {
                crate::Error::from(format!(
                    "failed to deserialize inventory for edition {:?}: {}",
                    edition, err
                ))
            })?;

            // Make the necessary calls indiscriminately:
            for (item_path, _hash) in &inventory {
                let content_hash = collection.locator_for(item_path.as_path()).hash();
                tokio::spawn(hubs().query(content_hash, QueryKind::Item).map(|_| ()));
            }

            return Ok(());
        }

        Err(crate::Error::from(format!(
            "Inventory not found for edition {:?}. Could not refresh",
            edition
        )))
    }
}
