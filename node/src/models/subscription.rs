use futures::prelude::*;
use futures::stream;
use rocksdb::{IteratorMode, WriteBatch};
use serde_derive::{Deserialize, Serialize};
use std::fmt::{self, Display};

use samizdat_common::rpc::QueryKind;
use samizdat_common::{ContentRiddle, Key};

use crate::db;
use crate::db::Table;
use crate::hubs;

use super::{Dropable, Edition, Inventory, SeriesRef};

#[derive(Debug, Serialize, Deserialize)]
pub enum SubscriptionKind {
    FullInventory,
}

impl Default for SubscriptionKind {
    fn default() -> SubscriptionKind {
        SubscriptionKind::FullInventory
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Subscription {
    public_key: Key,
    kind: SubscriptionKind,
}

impl Subscription {
    pub fn new(public_key: Key, kind: SubscriptionKind) -> Subscription {
        Subscription { public_key, kind }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubscriptionRef {
    pub public_key: Key,
}

impl Display for SubscriptionRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "subscription to {}", base64_url::encode(self.key()),)
    }
}

impl Dropable for SubscriptionRef {
    fn drop_if_exists_with(&self, batch: &mut WriteBatch) -> Result<(), crate::Error> {
        batch.delete_cf(Table::Subscriptions.get(), self.key());
        Ok(())
    }
}

impl SubscriptionRef {
    pub fn new(public_key: Key) -> SubscriptionRef {
        SubscriptionRef { public_key }
    }

    // pub fn public_key(&self) -> Key {
    //     self.public_key.clone()
    // }

    pub fn key(&self) -> &[u8] {
        self.public_key.as_bytes()
    }

    pub fn build(subscription: Subscription) -> Result<SubscriptionRef, crate::Error> {
        db().put_cf(
            Table::Subscriptions.get(),
            &subscription.public_key.as_bytes(),
            bincode::serialize(&subscription).expect("can serialize"),
        )?;

        Ok(SubscriptionRef {
            public_key: subscription.public_key,
        })
    }

    pub fn get(&self) -> Result<Option<Subscription>, crate::Error> {
        let maybe_value = db().get_cf(Table::Subscriptions.get(), &self.key())?;
        Ok(maybe_value
            .map(|value| bincode::deserialize(&value))
            .transpose()?)
    }

    pub fn get_all() -> Result<Vec<Subscription>, crate::Error> {
        db().iterator_cf(Table::Subscriptions.get(), IteratorMode::Start)
            .map(|(_, value)| Ok(bincode::deserialize(&value)?))
            .collect::<Result<Vec<_>, crate::Error>>()
    }

    pub fn find(riddle: &ContentRiddle) -> Option<SubscriptionRef> {
        let it = db().iterator_cf(Table::Subscriptions.get(), IteratorMode::Start);

        for (key, value) in it {
            match Key::from_bytes(&key) {
                Ok(key) => {
                    if riddle.resolves(&key.hash()) {
                        match bincode::deserialize(&value) {
                            Ok(subscription) => return Some(subscription),
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

    /// Reserved for future use.
    pub fn must_refresh(&self) -> Result<bool, crate::Error> {
        Ok(true)
    }

    /// Refresh the underlying series using and *already validated* edition.
    pub async fn refresh(&self, edition: Edition) -> Result<(), crate::Error> {
        let collection = edition.collection();
        let content_hash = collection.locator_for("_inventory".into()).hash();

        SeriesRef::new(edition.public_key().clone()).advance(&edition)?;

        if let Some(item) = hubs().query(content_hash, QueryKind::Item).await {
            if let Some(content) = item.content()? {
                let inventory: Inventory = serde_json::from_slice(&content).map_err(|err| {
                    crate::Error::from(format!(
                        "failed to deserialize inventory for edition {:?}: {}",
                        edition, err
                    ))
                })?;

                stream::iter(inventory.iter())
                    .for_each_concurrent(None, |(item_path, _hash)| {
                        let content_hash = collection.locator_for(item_path.as_path()).hash();
                        hubs().query(content_hash, QueryKind::Item).map(|_| ())
                    })
                    .await;

                return Ok(());
            }
        }

        Err(crate::Error::from(format!(
            "Inventory not found for edition {:?}. Could not refresh",
            edition
        )))
    }
}
